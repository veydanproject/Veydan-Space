<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { inspectorApp } from './inspector.svelte';
  import type { CustomGrid } from './inspector.svelte';

  interface Props {
    customGrid: CustomGrid;
  }

  let { customGrid }: Props = $props();

  function hexToRgba(hex: string, alpha: number): string {
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    return `rgba(${r},${g},${b},${alpha})`;
  }

  let gridStyle = $derived.by(() => {
    const size = inspectorApp.gridSize;
    if (size === 'off') return '';

    const color35 = hexToRgba(inspectorApp.gridColor, 0.35);
    const color15 = hexToRgba(inspectorApp.gridColor, 0.15);

    if (size === 'custom') {
      const { columns, gutter, margin } = customGrid;
      const colWidth = `calc((100vw - ${margin * 2}px - ${gutter * (columns - 1)}px) / ${columns})`;
      return `
        background-image:
          repeating-linear-gradient(
            to right,
            transparent 0,
            transparent ${margin}px,
            ${color15} ${margin}px,
            ${color15} calc(${margin}px + ${colWidth})
          );
        background-size: calc(${colWidth} + ${gutter}px) 100%;
        background-position: 0 0;
      `;
    }

    const px = size as number;
    return `
      background-image:
        linear-gradient(to right, ${color35} 1px, transparent 1px),
        linear-gradient(to bottom, ${color35} 1px, transparent 1px);
      background-size: ${px}px ${px}px;
    `;
  });
</script>

{#if inspectorApp.gridSize !== 'off'}
  <div class="inspector-grid" style={gridStyle} data-inspector-ui></div>
{/if}

<style>
  .inspector-grid {
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: 8001;
    image-rendering: pixelated;
  }
</style>
