<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Invisible edge/corner grips that drive window resizing. With server-side
  decorations off (decorations: false) the OS no longer provides resize borders,
  so we run our own via window.startResizeDragging(). Tauri only.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Window } from '@tauri-apps/api/window';

  // Not exported by @tauri-apps/api/window — derive it from the method signature.
  type ResizeDirection = Parameters<Window['startResizeDragging']>[0];

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let win: Window | null = null;

  onMount(() => {
    if (!isTauri) return;
    (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      win = getCurrentWindow();
    })();
  });

  function resize(dir: ResizeDirection, e: MouseEvent) {
    if (e.button !== 0 || !win) return;
    e.preventDefault();
    // Fire-and-forget; must be initiated during the mousedown so the compositor
    // hands over the interactive resize grab.
    win.startResizeDragging(dir);
  }
</script>

{#if isTauri}
  <div class="resize-layer">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="edge n" onmousedown={(e) => resize('North', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="edge s" onmousedown={(e) => resize('South', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="edge e" onmousedown={(e) => resize('East', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="edge w" onmousedown={(e) => resize('West', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="corner nw" onmousedown={(e) => resize('NorthWest', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="corner ne" onmousedown={(e) => resize('NorthEast', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="corner sw" onmousedown={(e) => resize('SouthWest', e)}></div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="corner se" onmousedown={(e) => resize('SouthEast', e)}></div>
  </div>
{/if}

<style>
  .resize-layer {
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: 9999;
  }
  .edge,
  .corner {
    position: absolute;
    pointer-events: auto;
  }
  .edge.n {
    top: 0;
    left: 6px;
    right: 6px;
    height: 4px;
    cursor: ns-resize;
  }
  .edge.s {
    bottom: 0;
    left: 6px;
    right: 6px;
    height: 4px;
    cursor: ns-resize;
  }
  .edge.e {
    right: 0;
    top: 6px;
    bottom: 6px;
    width: 4px;
    cursor: ew-resize;
  }
  .edge.w {
    left: 0;
    top: 6px;
    bottom: 6px;
    width: 4px;
    cursor: ew-resize;
  }
  .corner {
    width: 10px;
    height: 10px;
  }
  .corner.nw {
    top: 0;
    left: 0;
    cursor: nwse-resize;
  }
  .corner.ne {
    top: 0;
    right: 0;
    cursor: nesw-resize;
  }
  .corner.sw {
    bottom: 0;
    left: 0;
    cursor: nesw-resize;
  }
  .corner.se {
    bottom: 0;
    right: 0;
    cursor: nwse-resize;
  }
</style>
