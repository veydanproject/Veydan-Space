<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The 12 px gap between two panes that changes the width of one of them
  (design kit, «Граница колонки»). At rest a faint pill in the middle; under
  the pointer the pill grows in the accent and the tooltip tells what to do;
  while dragging an accent bar the whole height and the width in pixels. A
  double click goes back to the default; the keyboard moves the edge by
  16 px (← →, Home, End) when the pill has the focus. A computer's control:
  a phone screen never mounts it.

  `oninput` follows every step (the pane takes it at once); `oncommit` comes
  once at the end of a drag or a key, with `null` after a reset.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { clampWidth, dragWidth, keyWidth } from './paneWidth';

  interface Props {
    /** The width of the pane now, px. */
    value: number;
    min: number;
    max: number;
    /** What a double click goes back to. */
    defaultValue: number;
    /** Which side of the gap the pane is on. */
    side?: 'left' | 'right';
    oninput: (px: number) => void;
    oncommit: (px: number | null) => void;
  }
  let { value, min, max, defaultValue, side = 'right', oninput, oncommit }: Props = $props();

  let dragging = $state(false);
  let start = { x: 0, width: 0 };
  /** The pointer went past a click: a drag, kept at its end. */
  let moved = $state(false);

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture?.(e.pointerId);
    start = { x: e.clientX, width: value };
    moved = false;
    dragging = true;
    document.body.classList.add('pane-resizing');
  }

  function move(e: PointerEvent) {
    if (!dragging) return;
    if (!moved && Math.abs(e.clientX - start.x) < 2) return;
    moved = true;
    oninput(clampWidth(dragWidth(start.width, start.x, e.clientX, side), min, max));
  }

  function up(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    document.body.classList.remove('pane-resizing');
    (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
    if (moved) oncommit(clampWidth(value, min, max));
  }

  function reset() {
    oninput(clampWidth(defaultValue, min, max));
    oncommit(null);
  }

  function key(e: KeyboardEvent) {
    const next = keyWidth(value, e.key, side, min, max);
    if (next === null) return;
    e.preventDefault();
    if (next === value) return;
    oninput(next);
    oncommit(next);
  }

  $effect(() => () => document.body.classList.remove('pane-resizing'));
</script>

<!-- The gap takes the pointer; the pill is the control the keyboard and a screen reader reach. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pane-resizer"
  class:dragging
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  ondblclick={reset}
>
  <!-- A focusable separator is a control (WAI-ARIA window splitter); svelte reads `separator` as static. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="grip"
    role="separator"
    tabindex="0"
    aria-orientation="vertical"
    aria-label={$t('common_pane_width')}
    aria-valuenow={Math.round(value)}
    aria-valuemin={min}
    aria-valuemax={Math.max(min, max)}
    title={$t('common_pane_resize_hint')}
    onkeydown={key}
  >
    <span class="pill"></span>
  </div>
  {#if dragging && moved}<span class="badge">{Math.round(value)} px</span>{/if}
</div>

<style>
  .pane-resizer {
    position: relative;
    width: 12px;
    flex: none;
    height: 100%;
    cursor: col-resize;
    touch-action: none;
    background: transparent;
    transition: background 0.15s;
  }
  .grip {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 64px;
    transform: translateY(-50%);
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
    border-radius: 6px;
  }
  .pill {
    width: 4px;
    height: 28px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--text) 14%, transparent);
    transition: height 0.15s, background 0.15s;
  }
  .pane-resizer:hover { background: color-mix(in srgb, var(--accent-text) 6%, transparent); }
  .pane-resizer:hover .pill,
  .grip:focus-visible .pill { height: 44px; background: var(--accent-text); }
  .grip:focus-visible { box-shadow: 0 0 0 2px var(--accent-border); }

  .pane-resizer.dragging { background: color-mix(in srgb, var(--accent) 35%, transparent); }
  .pane-resizer.dragging .grip { top: 0; height: 100%; transform: none; }
  .pane-resizer.dragging .pill { height: 100%; border-radius: 0; background: #fff; transition: none; }

  .badge {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 3;
    white-space: nowrap;
    pointer-events: none;
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: #fff;
    background: var(--accent);
    border-radius: 8px;
    padding: 3px 8px;
  }

  :global(body.pane-resizing) { cursor: col-resize !important; user-select: none; }
  :global(body.pane-resizing iframe) { pointer-events: none; }

  @media (prefers-reduced-motion: reduce) {
    .pane-resizer, .pill { transition: none; }
  }
</style>
