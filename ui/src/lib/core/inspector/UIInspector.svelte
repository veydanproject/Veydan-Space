<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { portal } from '$lib/core/portal';
  import { inspectorApp } from './inspector.svelte';
  import InspectorToolbar from './InspectorToolbar.svelte';
  import InspectorGrid from './InspectorGrid.svelte';
  import InspectorRulers from './InspectorRulers.svelte';
  import InspectorGuides from './InspectorGuides.svelte';
  import InspectorHighlight from './InspectorHighlight.svelte';
  import InspectorInfoBox from './InspectorInfoBox.svelte';
  import { getElementAtPoint, getElementInfo } from './geometry';
  import type { ElementInfo } from './geometry';

  let hoveredInfo = $state<ElementInfo | null>(null);
  let mouseX = $state(0);
  let mouseY = $state(0);
  let highlightVisible = $state(true);

  $effect(() => {
    if (inspectorApp.enabled && inspectorApp.outlineAll) {
      document.body.classList.add('qa-outline-all');
    } else {
      document.body.classList.remove('qa-outline-all');
    }
  });

  $effect(() => {
    if (inspectorApp.enabled) {
      document.documentElement.style.setProperty('--qa-zoom', String(inspectorApp.zoom / 100));
    } else {
      document.documentElement.style.removeProperty('--qa-zoom');
    }
  });

  // Push layout down so the inspector toolbar doesn't cover app nav
  $effect(() => {
    if (inspectorApp.enabled) {
      document.documentElement.style.setProperty('--inspector-bar-height', 'var(--bar-h)');
    } else {
      document.documentElement.style.removeProperty('--inspector-bar-height');
    }
  });

  // Window-level listener so pointer-events: none on root doesn't block it
  $effect(() => {
    function onMouseMove(e: MouseEvent) {
      if (!inspectorApp.inspectMode) {
        hoveredInfo = null;
        return;
      }
      mouseX = e.clientX;
      mouseY = e.clientY;
      highlightVisible = false;
      const el = getElementAtPoint(e.clientX, e.clientY);
      highlightVisible = true;
      hoveredInfo = el ? getElementInfo(el) : null;
    }
    window.addEventListener('mousemove', onMouseMove);
    return () => window.removeEventListener('mousemove', onMouseMove);
  });
</script>

{#if inspectorApp.enabled}
  <div
    use:portal
    data-inspector-ui
    data-html2canvas-ignore
    class="inspector-root"
    role="presentation"
  >
    <InspectorToolbar />
    <InspectorGrid customGrid={inspectorApp.customGrid} />
    <InspectorRulers />
    <InspectorGuides />
    {#if highlightVisible && inspectorApp.inspectMode}
      <InspectorHighlight info={hoveredInfo} />
      <InspectorInfoBox info={hoveredInfo} {mouseX} {mouseY} />
    {/if}
  </div>
{/if}

<style>
  :global(.qa-outline-all *:not([data-inspector-ui]):not([data-inspector-ui] *)) {
    outline: 1px solid rgba(239, 68, 68, 0.4) !important;
  }

  .inspector-root {
    position: fixed;
    inset: 0;
    z-index: 8000;
    pointer-events: none;
  }
</style>
