<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onDestroy, type Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { onKeyboard } from '$lib/core/mobile/keyboard';
  import { t } from '$lib/core/mobile/i18n';
  import { backdropIn, backdropOut, sheetIn, sheetOut } from '$lib/core/ui/motion';
  import { portal } from '$lib/core/portal';

  interface Props {
    open: boolean;
    title?: string;
    onclose: () => void;
    onback?: () => void;
    /** The × of the head (default true); a question with its own «Отмена» has none (design kit, dialog on a phone). */
    closable?: boolean;
    /** Beside the title, in the head (a «?» of what the title means). */
    titleAside?: Snippet;
    /** Pulled up it grows to the whole screen (the head is then the grip), down it shrinks back, then closes. */
    expandable?: boolean;
    /** The name a screen reader says when the sheet has no title (default the title). */
    label?: string;
    children: Snippet;
    /** The sheet's actions: under the scrolling body, always in sight (the owner's rule: the buttons stay at the foot, the middle scrolls). */
    footer?: Snippet;
  }

  let { open, title = '', onclose, onback, closable = true, titleAside, expandable = false, label, children, footer }: Props = $props();

  let sheetEl = $state<HTMLDivElement>();
  let dragY = $state(0);
  let dragging = $state(false);
  let kb = $state(0);
  let viewH = $state(typeof window === 'undefined' ? 800 : window.innerHeight);
  let startY = 0;
  let startH = 0;
  let full = $state(false);

  const cap = $derived(Math.max(160, Math.round((viewH - kb) * 0.95)));

  $effect(() => {
    if (open) dragY = 0;
    else full = false;
  });

  $effect(() => {
    if (!open) return;
    return onKeyboard((n) => {
      kb = n;
      viewH = window.innerHeight;
    });
  });

  function onPointerDown(e: PointerEvent) {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragging = true;
    startY = e.clientY;
    startH = sheetEl?.offsetHeight ?? 200;
    dragY = 0;
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const dy = e.clientY - startY;
    // Up is only counted (the sheet stays on the bottom edge); down moves it.
    dragY = expandable && !full ? Math.max(-60, dy) : Math.max(0, dy);
  }

  function onPointerUp() {
    if (!dragging) return;
    dragging = false;
    const moved = dragY;
    dragY = 0;
    if (expandable && moved < -40 && !full) full = true;
    else if (moved > Math.min(120, startH * 0.25)) {
      if (full) full = false;
      else onclose();
    }
  }

  onDestroy(() => {
    dragging = false;
  });
</script>

{#if open}
  <!-- On the body: a sheet opened from inside a glass bar (backdrop-filter makes a box for `fixed`) would sit on the bar instead of the screen. -->
  <div class="m-dim" use:portal in:backdropIn out:backdropOut onclick={onclose} role="presentation"></div>
  <div
    bind:this={sheetEl}
    class="m-sheet"
    use:portal
    in:sheetIn
    out:sheetOut
    class:dragging
    class:full
    role="dialog"
    aria-modal="true"
    aria-label={label ?? title}
    style:transform="translateY({Math.max(0, dragY)}px)"
    style:bottom="{kb}px"
    style:max-height="{full ? viewH - kb : cap}px"
    style:height={full ? `${viewH - kb}px` : undefined}
    style:padding-bottom={kb > 0 ? '12px' : undefined}
  >
    <div
      class="handle"
      role="button"
      tabindex="0"
      aria-label={$t('common_close')}
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
    ></div>
    {#if title || onback}
      <div
        class="head"
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerUp}
      >
        {#if onback}
          <button type="button" class="m-ibtn" onclick={onback} aria-label={$t('common_back')} onpointerdown={(e) => e.stopPropagation()}>
            <Icon name="arrow-left" size={20} />
          </button>
        {/if}
        <!-- The «?» right after the title, outside the heading: its note is not read as part of the title. -->
        <div class="title-row">
          <h2>{title}</h2>
          {#if titleAside}<span class="aside" role="presentation" onpointerdown={(e) => e.stopPropagation()}>{@render titleAside()}</span>{/if}
        </div>
        {#if closable}<button type="button" class="m-ibtn" onclick={onclose} aria-label={$t('common_close')} onpointerdown={(e) => e.stopPropagation()}>
          <Icon name="x" size={20} />
        </button>{/if}
      </div>
    {/if}
    <div class="body">
      {@render children()}
    </div>
    <!-- Outside the body: it never scrolls away; the sheet's bottom padding keeps it above the safe area. -->
    {#if footer}<div class="foot">{@render footer()}</div>{/if}
  </div>
{/if}

<style>
  .m-sheet {
    height: auto;
    max-height: 95vh;
    overflow: hidden;
    transition: transform var(--dur-base) var(--ease-drawer);
  }
  .m-sheet.dragging { transition: none; }
  .m-sheet.full { border-radius: 0; padding-top: var(--sat); }
  .m-sheet.full .body { flex: 1 1 auto; }
  .handle {
    touch-action: none;
    padding: 10px 0 8px;
    margin-bottom: 0;
    width: 100%;
    height: auto;
    background: transparent;
    border-radius: 0;
    flex-shrink: 0;
  }
  .handle::after {
    content: '';
    display: block;
    width: 36px;
    height: 4px;
    border-radius: var(--radius-pill);
    background: var(--border-2); /* the kit's --track */
    margin: 0 auto;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-bottom: var(--sp-2);
    touch-action: none;
    flex-shrink: 0;
  }
  /* The title and what it keeps beside it: dragged with the head; the row a note opens under. */
  .title-row { position: relative; flex: 1; min-width: 0; display: flex; align-items: center; pointer-events: none; }
  h2 { min-width: 0; margin: 0; font-size: var(--fs-lg); font-weight: var(--fw-bold); }
  /* What the title keeps beside it (a «?»): pressed, not dragged; its note is placed by the title's row. */
  .aside { display: inline-flex; pointer-events: auto; }
  .body {
    flex: 0 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    touch-action: pan-y;
  }
  .body > :global(*) { flex-shrink: 0; }
  .foot { flex-shrink: 0; padding-top: var(--sp-3); }
</style>
