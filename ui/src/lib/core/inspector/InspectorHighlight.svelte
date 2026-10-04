<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { ElementInfo } from './geometry';

  interface Props {
    info: ElementInfo | null;
  }

  let { info }: Props = $props();
</script>

{#if info}
  <div
    class="highlight-box"
    style="
      top: {info.top}px;
      left: {info.left}px;
      width: {info.width}px;
      height: {info.height}px;
    "
    data-inspector-ui
  >
    <!-- margin overlay (blue) -->
    <div
      class="overlay-margin"
      style="
        top: {-info.marginTop}px;
        left: {-info.marginLeft}px;
        right: {-info.marginRight}px;
        bottom: {-info.marginBottom}px;
      "
    ></div>
    <!-- padding overlay (green) -->
    <div
      class="overlay-padding"
      style="
        top: 0; left: 0; right: 0; bottom: 0;
        border-top-width: {info.paddingTop}px;
        border-right-width: {info.paddingRight}px;
        border-bottom-width: {info.paddingBottom}px;
        border-left-width: {info.paddingLeft}px;
      "
    ></div>
  </div>
{/if}

<style>
  .highlight-box {
    position: fixed;
    z-index: 8005;
    pointer-events: none;
    outline: 2px solid rgba(99, 102, 241, 0.9);
    box-sizing: border-box;
  }

  .overlay-margin {
    position: absolute;
    background: rgba(251, 191, 36, 0.12);
    border: 1px dashed rgba(251, 191, 36, 0.5);
    pointer-events: none;
  }

  .overlay-padding {
    position: absolute;
    border-style: solid;
    border-color: rgba(34, 197, 94, 0.2);
    box-sizing: border-box;
    pointer-events: none;
  }
</style>
