<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { inspectorApp } from './inspector.svelte';

  function startDrag(e: MouseEvent, id: number, axis: 'h' | 'v') {
    if (e.button !== 0) return;
    e.preventDefault();
    document.body.style.userSelect = 'none';

    function onMove(ev: MouseEvent) {
      inspectorApp.moveGuide(id, axis === 'h' ? ev.clientY : ev.clientX);
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

{#if inspectorApp.showGuides}
  {#each inspectorApp.guides as guide (guide.id)}
    {#if guide.axis === 'h'}
      <div
        class="guide guide-h"
        style="top: {guide.position}px"
        role="presentation"
        onmousedown={(e) => startDrag(e, guide.id, 'h')}
        ondblclick={() => inspectorApp.removeGuide(guide.id)}
        data-inspector-ui
      ></div>
    {:else}
      <div
        class="guide guide-v"
        style="left: {guide.position}px"
        role="presentation"
        onmousedown={(e) => startDrag(e, guide.id, 'v')}
        ondblclick={() => inspectorApp.removeGuide(guide.id)}
        data-inspector-ui
      ></div>
    {/if}
  {/each}
{/if}

<style>
  .guide {
    position: fixed;
    z-index: 8009;
    pointer-events: auto;
    cursor: grab;
  }

  .guide:active {
    cursor: grabbing;
  }

  .guide-h {
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(239, 68, 68, 0.8);
    border-top: 1px dashed rgba(239, 68, 68, 0.6);
    cursor: ns-resize;
  }

  .guide-v {
    top: 0;
    bottom: 0;
    width: 1px;
    background: rgba(239, 68, 68, 0.8);
    border-left: 1px dashed rgba(239, 68, 68, 0.6);
    cursor: ew-resize;
  }
</style>
