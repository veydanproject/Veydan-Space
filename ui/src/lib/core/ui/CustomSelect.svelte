<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The one dropdown list of the app (design kit, Kit-06-custom-select). The field is a text field
  of 40 (44 on a phone); where its place lets it, as wide as its longest row and the check's column,
  up to 240; a longer value ends in «…» and the field's tooltip says it whole. The list is the glass
  of ContextMenu, 4 under the field, exactly as wide as the field, edge on edge; its rows 36, eight of
  them without a scroll, the text on the field's, a longer row in two lines; the chosen row is 500
  with a check in a column of its own on the right, on its first line, no fill. ↓/↑ (round),
  Home/End and the first letters move between the rows, Enter picks one, Escape closes and gives the
  focus back to the field. On a phone the rows are a bottom sheet.
-->
<script lang="ts">
  import { onMount, onDestroy, tick, untrack } from 'svelte';
  import { portal } from '$lib/core/portal';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { holdBack } from './sheetBack';

  export interface SelectOption {
    label: string;
    value: string | null | undefined;
    disabled?: boolean;
  }

  interface Props {
    options: SelectOption[];
    value: string | null | undefined;
    placeholder?: string;
    id?: string;
    onchange?: (value: string | null | undefined) => void;
    disabled?: boolean;
    ariaLabel?: string;
    title?: string;
    /** A field in error: its frame is the danger colour (aria-invalid). */
    invalid?: boolean;
    /** A selector of an ancestor the list is laid by instead of the field: a group the field is part of. */
    anchor?: string;
  }

  let {
    options, value = $bindable(), placeholder = '—', id = '', onchange,
    disabled = false, ariaLabel, title, invalid = false, anchor,
  }: Props = $props();

  const uid = $props.id();
  const ROW = 36;

  let open = $state(false);
  let triggerEl = $state<HTMLButtonElement | null>(null);
  let dropdownEl = $state<HTMLDivElement | null>(null);
  let dropdownStyle = $state('');
  let textEl = $state<HTMLSpanElement | null>(null);
  let clipped = $state(false);
  let sheetTitle = $state('');
  let buf = '';
  let bufTimer: ReturnType<typeof setTimeout> | undefined;

  let selected = $derived(options.find(o => o.value === value));

  async function show() {
    if (disabled || open) return;
    if (isMobile) {
      // The sheet is titled by the field's label (or its name for a screen reader).
      // A label around the field holds its value too: only the label's own words.
      const own = triggerEl?.labels?.[0]?.cloneNode(true) as HTMLElement | undefined;
      own?.querySelectorAll('button, [role="listbox"]').forEach((n) => n.remove());
      const label = own?.textContent?.replace(/\s+/g, ' ').trim();
      sheetTitle = label || ariaLabel || title || '';
      open = true;
      return;
    }
    const rect = ((anchor && triggerEl?.closest(anchor)) || triggerEl)?.getBoundingClientRect();
    if (!rect) return;
    dropdownStyle = `top:${rect.bottom + 7}px;left:${rect.left}px;width:${rect.width}px;visibility:hidden`;
    open = true;
    await tick();
    position(rect);
    const rows = enabled();
    const target = rows.find(b => b.getAttribute('aria-selected') === 'true') ?? rows[0];
    (target ?? dropdownEl)?.focus();
  }

  function position(rect: DOMRect) {
    if (!dropdownEl) return;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const h = dropdownEl.offsetHeight;
    // Edge on edge with the field, its width unrounded: a row as wide as the field's text fits it.
    const w = rect.width;
    let left = rect.left;
    if (left + w > vw - 4) left = vw - w - 4;
    if (left < 4) left = 4;
    const below = vh - rect.bottom - 11;
    const above = rect.top - 11;
    // Under the field; over it when it does not fit below and there is more room above.
    const up = h > below && above > below;
    const room = Math.max(ROW + 10, up ? above : below);
    const top = up ? Math.max(4, rect.top - 7 - Math.min(h, room)) : rect.bottom + 7;
    dropdownStyle = `top:${top}px;left:${left}px;width:${w}px;--room:${room}px`;
  }

  function close(refocus = false) {
    open = false;
    if (refocus) triggerEl?.focus({ preventScroll: true });
  }

  function toggle() {
    if (open) close();
    else show();
  }

  function select(opt: SelectOption) {
    if (opt.disabled) return;
    value = opt.value;
    onchange?.(opt.value);
    close(!isMobile);
  }

  const enabled = () => [...(dropdownEl?.querySelectorAll<HTMLButtonElement>('.option:not(:disabled)') ?? [])];

  function navigate(e: KeyboardEvent): boolean {
    const list = enabled();
    if (!list.length) return false;
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End') {
      const n = e.key === 'Home' ? 0
        : e.key === 'End' ? list.length - 1
        : e.key === 'ArrowDown' ? (i + 1) % list.length
        : i < 0 ? list.length - 1 : (i - 1 + list.length) % list.length;
      list[n].focus();
      return true;
    }
    if (e.key.length === 1 && /\S/.test(e.key) && !e.ctrlKey && !e.metaKey && !e.altKey) {
      clearTimeout(bufTimer);
      buf += e.key.toLocaleLowerCase();
      bufTimer = setTimeout(() => (buf = ''), 500);
      const from = i < 0 ? 0 : i + (buf.length === 1 ? 1 : 0);
      const order = [...list.slice(from), ...list.slice(0, from)];
      const hit = order.find((b) => (b.querySelector('.option-label')?.textContent ?? '').trim().toLocaleLowerCase().startsWith(buf));
      hit?.focus();
      return true;
    }
    return false;
  }

  function onTriggerKeydown(e: KeyboardEvent) {
    if (open || disabled) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      show();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === 'Escape') {
      e.stopPropagation();
      e.preventDefault();
      close(!isMobile);
      return;
    }
    if (isMobile) return;
    // The list lives on the body: Tab goes on from the field, not from the end of the page.
    if (e.key === 'Tab') { close(true); return; }
    if (navigate(e)) { e.preventDefault(); e.stopPropagation(); }
  }

  function onOutsideClick(e: MouseEvent) {
    if (!open || isMobile) return;
    const t = e.target as Node;
    if (triggerEl && !triggerEl.contains(t) && dropdownEl && !dropdownEl.contains(t)) close();
  }

  // A scroll of the page or of a parent of the field leaves the fixed list behind: it closes.
  // A scroll of the list itself is its own.
  function onScroll(e: Event) {
    const t = e.target;
    if (dropdownEl && t instanceof Node && dropdownEl.contains(t)) return;
    close();
  }

  $effect(() => {
    if (disabled && open) close();
  });
  $effect(() => {
    if (!open || isMobile) return;
    document.addEventListener('scroll', onScroll, true);
    return () => document.removeEventListener('scroll', onScroll, true);
  });
  // The system «Назад» closes the sheet only, not the page under it.
  $effect(() => {
    if (!open || !isMobile) return;
    // Untracked: taking a place in the history writes the page's state, which it also reads.
    return untrack(() => holdBack(() => close(), 'selectSheet'));
  });

  onMount(() => {
    document.addEventListener('mousedown', onOutsideClick, true);
    document.addEventListener('keydown', onKeydown, true);
  });
  onDestroy(() => {
    document.removeEventListener('mousedown', onOutsideClick, true);
    document.removeEventListener('keydown', onKeydown, true);
    clearTimeout(bufTimer);
  });
</script>

<button
  bind:this={triggerEl}
  type="button"
  id={id || undefined}
  class="trigger"
  role="combobox"
  class:open
  class:mobile={isMobile}
  class:placeholder={!selected}
  {disabled}
  title={clipped && selected ? selected.label : title}
  aria-label={ariaLabel}
  aria-invalid={invalid || undefined}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls="{uid}-list"
  onclick={toggle}
  onkeydown={onTriggerKeydown}
  onpointerenter={() => (clipped = !!textEl && textEl.scrollWidth > textEl.clientWidth)}
>
  <!-- Every row's label, unseen, in the label's cell: a field as wide as its words is as wide as the longest. -->
  <span class="trigger-label">
    <span class="trigger-text" bind:this={textEl}>{selected?.label ?? placeholder}</span>
    {#each options as opt, i (i)}<span class="trigger-sizer" aria-hidden="true">{opt.label}</span>{/each}
    {#if !selected}<span class="trigger-sizer" aria-hidden="true">{placeholder}</span>{/if}
  </span>
  <svg class="chevron" class:rotated={open} width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
</button>

{#if isMobile}
  {#if open}
    <BottomSheet open title={sheetTitle} label={sheetTitle || undefined} closable={false} onclose={() => close()}>
      <div class="sheet-list" id="{uid}-list" role="listbox" aria-label={sheetTitle || undefined}>
        {#each options as opt, i (i)}
          <button
            type="button"
            class="sheet-option"
            class:selected={opt.value === value}
            role="option"
            aria-selected={opt.value === value}
            disabled={opt.disabled}
            onclick={() => select(opt)}
          >
            <span class="option-label">{opt.label}</span>
            {#if opt.value === value}<Icon name="check" size={20} />{/if}
          </button>
        {/each}
      </div>
    </BottomSheet>
  {/if}
{:else if open}
  <div
    bind:this={dropdownEl}
    use:portal
    class="dropdown"
    id="{uid}-list"
    style={dropdownStyle}
    role="listbox"
    tabindex="-1"
    aria-label={ariaLabel ?? title}
  >
    {#each options as opt, i (i)}
      <button
        type="button"
        class="option"
        class:selected={opt.value === value}
        role="option"
        aria-selected={opt.value === value}
        disabled={opt.disabled}
        tabindex="-1"
        onclick={() => select(opt)}
      >
        <span class="option-label">{opt.label}</span>
        <span class="option-check">{#if opt.value === value}<Icon name="check" size={16} />{/if}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  /* The field: a text field of the forms (base.css input), with a chevron. */
  .trigger {
    --sel-border: var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: var(--control-h);
    padding: 0 0.75rem;
    background: var(--pane-strong);
    border: 1px solid var(--sel-border);
    border-radius: var(--radius);
    color: var(--text);
    font-size: var(--fs-base);
    font-family: inherit;
    cursor: pointer;
    text-align: left;
    gap: var(--sp-2);
    outline: none;
    transition: border-color var(--dur-fast), box-shadow var(--dur-fast);
  }
  /* Rest < dark rest < hover < error < focus: the first three weigh the same, the order decides. */
  :global(:where([data-theme='dark'])) .trigger { --sel-border: var(--pane-strong-border); }
  .trigger:where(:hover:not(:disabled)) { --sel-border: var(--border-2); }
  .trigger[aria-invalid='true'] { --sel-border: var(--danger); }
  /* Open or clicked: the border only. The ring comes with the keyboard, and the list opens 7 below (3 of it + 4). */
  .trigger:focus-visible, .trigger.open {
    outline: none;
    --sel-border: var(--accent-border);
  }
  .trigger:focus-visible:not(.open) { box-shadow: 0 0 0 3px var(--accent-bg); }
  .trigger:disabled { opacity: 0.45; cursor: not-allowed; }
  .trigger.mobile { height: auto; min-height: var(--control-h-lg); font-size: var(--fs-md); }
  .trigger-label {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
  }
  .trigger-text, .trigger-sizer {
    grid-area: 1 / 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The shown text takes the cell and gives it no width of its own: a chosen row longer than 240
     ends in «…» there, the field as wide as the sizers say. */
  .trigger-text { contain: inline-size; }
  /* 500 like the chosen row of the list, and the room of the check's column (16 and the gap of 8):
     the row fits the list whichever is chosen. The field by its words is up to 240 (240 - 46 of its
     frame, padding, gap and chevron); a form's field is as wide as its place. */
  .trigger-sizer {
    visibility: hidden;
    font-weight: var(--fw-medium);
    box-sizing: content-box;
    padding-right: calc(16px + var(--sp-2));
    max-width: calc(240px - 46px - 16px - var(--sp-2));
  }
  .trigger.placeholder .trigger-label { color: var(--text-3); }
  :global([data-theme='dark']) .trigger.placeholder .trigger-label { color: var(--text-2); }
  .chevron {
    flex-shrink: 0;
    color: var(--text-2);
    transition: transform var(--dur-fast);
  }
  .chevron.rotated { transform: rotate(180deg); }

  /* The list: the glass of ContextMenu (.context-menu), one layer. */
  .dropdown {
    --opt-hover: var(--surface-hover);
    position: fixed;
    z-index: var(--z-popover);
    /* Eight rows of 36 and the inset of 4 above and below, inside the 1px frame; past that it scrolls. */
    max-height: min(calc(8 * 36px + 8px + 2px), var(--room, 100vh));
    overflow-y: auto;
    overflow-x: hidden;
    padding: 4px;
    background: var(--glass-bg);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-md);
    box-shadow: var(--glass-highlight), var(--shadow-lg);
    -webkit-backdrop-filter: var(--glass-filter);
    backdrop-filter: var(--glass-filter);
    outline: none;
    transform-origin: top left;
    animation: list-in var(--dur-fast) var(--ease) both;
  }
  :global([data-theme='dark']) .dropdown { --opt-hover: var(--pane-strong-border); }
  @keyframes list-in {
    from { opacity: 0; transform: scale(0.96); }
    to   { opacity: 1; transform: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .dropdown { animation: none; }
  }

  /* A row: the row of a menu (.menu-item). Its text 13 from the list's edge, as in the field
     (1 + 4 + 8); the check's column 16 on the right, empty on the other rows, on the first line
     of a row of two. */
  .option {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    width: 100%;
    min-height: 36px;
    padding: var(--sp-2) var(--sp-1) var(--sp-2) var(--sp-2);
    line-height: 20px;
    border-radius: var(--radius-sm);
    background: transparent;
    border: none;
    color: var(--text);
    font-size: var(--fs-base);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    outline: none;
  }
  .option:hover:not(:disabled),
  .option:focus:not(:disabled) { background: var(--opt-hover); outline: none; }
  .option:focus-visible { box-shadow: inset 0 0 0 1.5px var(--focus-ring); }
  .option:disabled { color: var(--text-3); cursor: default; }
  .option.selected { font-weight: var(--fw-medium); }
  .option-check { display: flex; align-items: center; flex: 0 0 16px; width: 16px; height: 20px; }
  .option-check :global(svg) { color: var(--text); }
  /* A longer row: two lines, the second ending in «…». */
  .option-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  /* The phone's sheet: rows of 44 (ContextMenu's sheet, the rows of a choice). */
  .sheet-list { display: flex; flex-direction: column; }
  .sheet-option {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    width: 100%;
    min-height: 44px;
    padding: 0 var(--sp-2);
    background: none;
    border: none;
    border-radius: var(--radius-md);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-md);
    text-align: left;
  }
  .sheet-option:active:not(:disabled) { background: var(--surface-2); }
  .sheet-option:disabled { color: var(--text-3); }
  .sheet-option.selected { font-weight: var(--fw-medium); }
  .sheet-option :global(svg) { flex-shrink: 0; color: var(--text); }
</style>
