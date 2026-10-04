<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { inspectorApp } from './inspector.svelte';

  const RULER_SIZE = 20;
  const TICK_STEP = 10;
  const LABEL_STEP = 50;

  let viewW = $state(window.innerWidth);
  let viewH = $state(window.innerHeight);

  $effect(() => {
    function onResize() {
      viewW = window.innerWidth;
      viewH = window.innerHeight;
    }
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  function hTicks(width: number) {
    const ticks: { x: number; label: boolean }[] = [];
    for (let i = RULER_SIZE; i < width; i += TICK_STEP) {
      ticks.push({ x: i, label: i % LABEL_STEP === 0 });
    }
    return ticks;
  }

  function vTicks(height: number) {
    const ticks: { y: number; label: boolean }[] = [];
    for (let i = RULER_SIZE; i < height; i += TICK_STEP) {
      ticks.push({ y: i, label: i % LABEL_STEP === 0 });
    }
    return ticks;
  }

  function startDragH(e: MouseEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    document.body.style.userSelect = 'none';

    inspectorApp.addGuide('h', e.clientY);
    inspectorApp.showGuides = true;
    const guideId = inspectorApp.guides[inspectorApp.guides.length - 1].id;

    function onMove(ev: MouseEvent) {
      inspectorApp.moveGuide(guideId, ev.clientY);
    }

    function onUp() {
      document.body.style.userSelect = '';
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    }

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  function startDragV(e: MouseEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    document.body.style.userSelect = 'none';

    inspectorApp.addGuide('v', e.clientX);
    inspectorApp.showGuides = true;
    const guideId = inspectorApp.guides[inspectorApp.guides.length - 1].id;

    function onMove(ev: MouseEvent) {
      inspectorApp.moveGuide(guideId, ev.clientX);
    }

    function onUp() {
      document.body.style.userSelect = '';
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    }

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }
</script>

{#if inspectorApp.showRulers}
  <!-- Horizontal ruler (top) -->
  <div
    class="ruler ruler-h"
    role="presentation"
    onmousedown={startDragH}
    data-inspector-ui
  >
    <svg width={viewW} height={RULER_SIZE}>
      {#each hTicks(viewW) as tick}
        <line
          x1={tick.x}
          y1={tick.label ? 0 : RULER_SIZE - 6}
          x2={tick.x}
          y2={RULER_SIZE}
          stroke="rgba(99,102,241,.6)"
          stroke-width="1"
        />
        {#if tick.label}
          <text x={tick.x + 2} y={RULER_SIZE - 4} font-size="8" fill="rgba(99,102,241,.8)">
            {tick.x - RULER_SIZE}
          </text>
        {/if}
      {/each}
    </svg>
  </div>

  <!-- Vertical ruler (left) -->
  <div
    class="ruler ruler-v"
    role="presentation"
    onmousedown={startDragV}
    data-inspector-ui
  >
    <svg width={RULER_SIZE} height={viewH}>
      {#each vTicks(viewH) as tick}
        <line
          x1={tick.label ? 0 : RULER_SIZE - 6}
          y1={tick.y}
          x2={RULER_SIZE}
          y2={tick.y}
          stroke="rgba(99,102,241,.6)"
          stroke-width="1"
        />
        {#if tick.label}
          <text
            x={RULER_SIZE - 3}
            y={tick.y - 2}
            font-size="8"
            fill="rgba(99,102,241,.8)"
            transform={`rotate(-90, ${RULER_SIZE - 3}, ${tick.y - 2})`}
          >
            {tick.y - RULER_SIZE}
          </text>
        {/if}
      {/each}
    </svg>
  </div>

  <!-- Corner square -->
  <div class="ruler-corner" data-inspector-ui></div>
{/if}

<style>
  .ruler {
    position: fixed;
    z-index: 8010;
    background: rgba(15, 23, 42, 0.85);
    cursor: crosshair;
    pointer-events: auto;
  }

  .ruler-h {
    top: var(--bar-h);
    left: 0;
    right: 0;
    height: 20px;
  }

  .ruler-v {
    top: var(--bar-h);
    left: 0;
    bottom: 0;
    width: 20px;
  }

  .ruler-corner {
    position: fixed;
    top: var(--bar-h);
    left: 0;
    width: 20px;
    height: 20px;
    background: rgba(30, 41, 59, 0.95);
    z-index: 8011;
    pointer-events: none;
  }
</style>
