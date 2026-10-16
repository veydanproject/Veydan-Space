<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Generic right-click menu. Positioned at a viewport coordinate and clamped to
  stay on screen; closes on outside-click, Escape, Tab, or after an item fires.
  Modelled on CustomSelect's fixed-positioned popover. On a phone the same
  items are a bottom sheet (the long press that asked for it places nothing).
  Design kit, menu: grows from the point in 150 ms and fades in 100 ms (M05);
  ↓/↑ (round), Home/End and the first letters move between the items, Enter
  fires one, Escape gives the focus back to where it was.
-->
<script lang="ts">
  import { onMount, onDestroy, type Snippet } from 'svelte';
  import { portal } from '$lib/core/portal';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { menuOut } from './motion';

  export interface MenuItem {
    type?: 'item';
    label: string;
    icon?: string;
    danger?: boolean;
    disabled?: boolean;
    shortcut?: string;
    onselect: () => void;
  }
  export type MenuEntry = MenuItem | { type: 'separator' };

  interface Props {
    open: boolean;
    x: number;
    y: number;
    items: MenuEntry[];
    onclose: () => void;
    /** Optional header rendered above the items (e.g. the target name). */
    header?: Snippet;
    /** 'point': its corner at (x, y), the default; 'above': centred over (x, y), 8 above it (below when there is no room). */
    place?: 'point' | 'above';
  }

  let { open = $bindable(), x, y, items, onclose, header, place = 'point' }: Props = $props();

  let menuEl = $state<HTMLDivElement | null>(null);
  let style = $state('');
  let origin = $state('top left');
  let returnFocus: HTMLElement | null = null;
  let buf = '';
  let bufTimer: ReturnType<typeof setTimeout> | undefined;

  function reposition() {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const w = menuEl?.offsetWidth ?? 220;
    const h = menuEl?.offsetHeight ?? 240;
    let left = place === 'above' ? x - w / 2 : x;
    let top = place === 'above' ? (y - h - 8 >= 4 ? y - h - 8 : y + 8) : y;
    if (left < 4) left = 4;
    if (left + w > vw - 4) left = vw - w - 4;
    if (top + h > vh - 4) top = Math.max(4, vh - h - 4);
    style = `left:${left}px;top:${top}px`;
    origin = `${Math.round(x - left)}px ${Math.round(y - top)}px`;
  }

  $effect(() => {
    if (open) {
      // measure after render, then clamp
      style = `left:${x}px;top:${y}px;visibility:hidden`;
      requestAnimationFrame(() => {
        reposition();
        if (isMobile || !menuEl) return;
        // The keys go to the menu; the focus comes back where it was when it closes.
        const active = document.activeElement;
        returnFocus = active instanceof HTMLElement && active !== document.body ? active : null;
        menuEl.focus({ preventScroll: true });
      });
    }
  });

  const enabled = () => [...(menuEl?.querySelectorAll<HTMLButtonElement>('.menu-item:not(:disabled)') ?? [])];

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
      const hit = order.find((b) => (b.querySelector('.menu-label')?.textContent ?? '').trim().toLocaleLowerCase().startsWith(buf));
      hit?.focus();
      return true;
    }
    return false;
  }

  // The finger that held for the menu may lift over the sheet's backdrop:
  // that lift is not a tap that dismisses the sheet.
  let openedAt = 0;
  $effect(() => {
    if (open && isMobile) openedAt = performance.now();
  });
  function dismissSheet() {
    if (performance.now() - openedAt > 350) close();
  }

  function close(refocus = false) {
    open = false;
    onclose();
    if (refocus) returnFocus?.focus({ preventScroll: true });
    returnFocus = null;
  }

  function fire(item: MenuItem) {
    if (item.disabled) return;
    close();
    item.onselect();
  }

  function onKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === 'Escape') {
      e.stopPropagation();
      close(true);
      return;
    }
    if (isMobile) return;
    if (e.key === 'Tab') { close(); return; }
    if (navigate(e)) { e.preventDefault(); e.stopPropagation(); }
  }
  function onOutside(e: MouseEvent) {
    if (open && menuEl && !menuEl.contains(e.target as Node)) close();
  }

  onMount(() => {
    document.addEventListener('mousedown', onOutside, true);
    document.addEventListener('contextmenu', onOutside, true);
    document.addEventListener('keydown', onKeydown, true);
  });
  onDestroy(() => {
    document.removeEventListener('mousedown', onOutside, true);
    document.removeEventListener('contextmenu', onOutside, true);
    document.removeEventListener('keydown', onKeydown, true);
  });
</script>

{#if isMobile && open}
  <!-- On a phone the menu is a bottom sheet of finger-sized rows, like every
       other choice there; where it was asked for does not place it. -->
  <div class="sheet-host" use:portal>
    <BottomSheet open={!!open} title="" onclose={dismissSheet}>
      {#if header}<div class="sheet-menu-header">{@render header()}</div>{/if}
      <div class="sheet-menu" role="menu">
        {#each items as item (item)}
          {#if item.type === 'separator'}
            <div class="sheet-sep"></div>
          {:else}
            <button
              class="sheet-item"
              class:danger={item.danger}
              disabled={item.disabled}
              role="menuitem"
              onclick={() => fire(item)}
            >
              {#if item.icon}<Icon name={item.icon} size={20} />{:else}<span class="sheet-icon-gap"></span>{/if}
              <span class="menu-label">{item.label}</span>
            </button>
          {/if}
        {/each}
      </div>
    </BottomSheet>
  </div>
{:else if open && !isMobile}
  <div bind:this={menuEl} class="context-menu" style={style} style:transform-origin={origin} use:portal role="menu" tabindex="-1" out:menuOut>
    {#if header}
      <div class="menu-header">{@render header()}</div>
    {/if}
    {#each items as item (item)}
      {#if item.type === 'separator'}
        <div class="menu-sep"></div>
      {:else}
        <button
          class="menu-item"
          class:danger={item.danger}
          disabled={item.disabled}
          role="menuitem"
          onclick={() => fire(item)}
        >
          {#if item.icon}<Icon name={item.icon} size={14} />{:else}<span class="icon-gap"></span>{/if}
          <span class="menu-label">{item.label}</span>
          {#if item.shortcut}<span class="menu-shortcut">{item.shortcut}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .context-menu {
    position: fixed;
    z-index: var(--z-popover);
    min-width: 200px;
    max-width: 320px;
    padding: 4px;
    background: var(--glass-bg);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-md);
    box-shadow: var(--glass-highlight), var(--shadow-lg);
    -webkit-backdrop-filter: var(--glass-filter);
    backdrop-filter: var(--glass-filter);
    outline: none;
    animation: menu-in var(--dur-fast) var(--ease) both;
  }
  @keyframes menu-in {
    from { opacity: 0; transform: scale(0.96); }
    to   { opacity: 1; transform: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .context-menu { animation: none; }
  }

  .menu-header {
    padding: var(--sp-1) var(--sp-2) var(--sp-2);
    margin-bottom: 4px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .menu-item {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    min-height: 36px;
    padding: var(--sp-1) var(--sp-3);
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text);
    font-size: var(--fs-base);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover:not(:disabled),
  .menu-item:focus:not(:disabled) { background: var(--surface-hover); outline: none; }
  .menu-item:focus-visible { box-shadow: inset 0 0 0 1.5px var(--accent-border); }
  .menu-item:disabled { color: var(--text-3); cursor: default; }
  .menu-item.danger { color: var(--danger-text); }
  .menu-item.danger:hover:not(:disabled),
  .menu-item.danger:focus:not(:disabled) { background: var(--danger-bg); }

  .menu-item :global(svg) { flex-shrink: 0; color: var(--text-2); }
  .menu-item.danger :global(svg) { color: var(--danger-text); }
  .icon-gap { width: 14px; flex-shrink: 0; }

  .menu-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-shortcut {
    flex-shrink: 0;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
  }

  .menu-sep {
    height: 1px;
    margin: 4px 2px;
    background: var(--border);
  }

  /* The phone's sheet: rows of the shell's lists (mobile.css .m-row), 52px high. */
  .sheet-menu-header {
    padding: 0 var(--sp-1) var(--sp-2);
    color: var(--text-2);
    font-size: var(--fs-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sheet-menu { display: flex; flex-direction: column; }
  .sheet-item {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    width: 100%;
    min-height: 52px;
    padding: 0 var(--sp-2);
    background: none;
    border: none;
    border-radius: var(--radius-md);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-md);
    text-align: left;
  }
  .sheet-item:active:not(:disabled) { background: var(--surface-2); }
  .sheet-item:disabled { color: var(--text-3); }
  .sheet-item.danger { color: var(--danger-text); }
  .sheet-item :global(svg) { flex-shrink: 0; color: var(--text-2); }
  .sheet-item.danger :global(svg) { color: var(--danger-text); }
  .sheet-icon-gap { width: 20px; flex-shrink: 0; }
  .sheet-sep { height: 1px; margin: var(--sp-1) var(--sp-2); background: var(--border); flex-shrink: 0; }
</style>
