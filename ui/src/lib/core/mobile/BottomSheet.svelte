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
    children: Snippet;
  }

  let { open, title = '', onclose, onback, closable = true, children }: Props = $props();

  let sheetEl = $state<HTMLDivElement>();
  let dragY = $state(0);
  let dragging = $state(false);
  let kb = $state(0);
  let viewH = $state(typeof window === 'undefined' ? 800 : window.innerHeight);
  let startY = 0;
  let startH = 0;

  const cap = $derived(Math.max(160, Math.round((viewH - kb) * 0.95)));

  $effect(() => {
    if (open) dragY = 0;
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
    dragY = Math.max(0, e.clientY - startY);
  }

  function onPointerUp() {
    if (!dragging) return;
    dragging = false;
    if (dragY > Math.min(120, startH * 0.25)) onclose();
    else dragY = 0;
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
    role="dialog"
    aria-modal="true"
    aria-label={title}
    style:transform="translateY({dragY}px)"
    style:bottom="{kb}px"
    style:max-height="{cap}px"
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
        <h2>{title}</h2>
        {#if closable}<button type="button" class="m-ibtn" onclick={onclose} aria-label={$t('common_close')} onpointerdown={(e) => e.stopPropagation()}>
          <Icon name="x" size={20} />
        </button>{/if}
      </div>
    {/if}
    <div class="body">
      {@render children()}
    </div>
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
  h2 { flex: 1; margin: 0; font-size: var(--fs-lg); font-weight: var(--fw-bold); pointer-events: none; }
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
</style>
