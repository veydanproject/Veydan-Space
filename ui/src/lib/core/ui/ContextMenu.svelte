<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Generic right-click menu. Positioned at a viewport coordinate and clamped to
  stay on screen; closes on outside-click, Escape, or after an item fires.
  Modelled on CustomSelect's fixed-positioned popover. On a phone the same
  items are a bottom sheet (the long press that asked for it places nothing).
-->
<script lang="ts">
  import { onMount, onDestroy, type Snippet } from 'svelte';
  import { portal } from '$lib/core/portal';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

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
  }

  let { open = $bindable(), x, y, items, onclose, header }: Props = $props();

  let menuEl = $state<HTMLDivElement | null>(null);
  let style = $state('');

  function reposition() {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const w = menuEl?.offsetWidth ?? 220;
    const h = menuEl?.offsetHeight ?? 240;
    let left = x;
    let top = y;
    if (left + w > vw - 4) left = vw - w - 4;
    if (top + h > vh - 4) top = Math.max(4, vh - h - 4);
    style = `left:${left}px;top:${top}px`;
  }

  $effect(() => {
    if (open) {
      // measure after render, then clamp
      style = `left:${x}px;top:${y}px;visibility:hidden`;
      requestAnimationFrame(reposition);
    }
  });

  // The finger that held for the menu may lift over the sheet's backdrop:
  // that lift is not a tap that dismisses the sheet.
  let openedAt = 0;
  $effect(() => {
    if (open && isMobile) openedAt = performance.now();
  });
  function dismissSheet() {
    if (performance.now() - openedAt > 350) close();
  }

  function close() {
    open = false;
    onclose();
  }

  function fire(item: MenuItem) {
    if (item.disabled) return;
    close();
    item.onselect();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) {
      e.stopPropagation();
      close();
    }
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
  <div bind:this={menuEl} class="context-menu" style={style} use:portal role="menu">
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
    background: var(--surface-drawer);
    border: 1px solid var(--border-2);
    border-radius: var(--radius);
    box-shadow: var(--shadow-lg);
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
    padding: var(--sp-2) var(--sp-2);
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text);
    font-size: var(--fs-sm);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover:not(:disabled) { background: var(--surface-hover); }
  .menu-item:disabled { color: var(--text-3); cursor: default; }
  .menu-item.danger { color: var(--danger-text); }
  .menu-item.danger:hover:not(:disabled) { background: var(--danger-bg); }

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
    font-size: 13px;
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
    font-size: 16px;
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
