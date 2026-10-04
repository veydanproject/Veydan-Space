<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { ElementInfo } from './geometry';

  interface Props {
    info: ElementInfo | null;
    mouseX: number;
    mouseY: number;
  }

  let { info, mouseX, mouseY }: Props = $props();

  const OFFSET = 14;
  const BOX_W = 220;
  const BOX_H = 130;

  let boxLeft = $derived(
    mouseX + OFFSET + BOX_W > window.innerWidth ? mouseX - BOX_W - OFFSET : mouseX + OFFSET
  );
  let boxTop = $derived(
    mouseY + OFFSET + BOX_H > window.innerHeight ? mouseY - BOX_H - OFFSET : mouseY + OFFSET
  );
</script>

{#if info}
  <div
    class="info-box"
    style="left: {boxLeft}px; top: {boxTop}px;"
    data-inspector-ui
  >
    <div class="tag">{info.tagName}{info.id ? `#${info.id}` : ''}</div>
    {#if info.classList}
      <div class="cls">.{info.classList.replace(/ /g, '.')}</div>
    {/if}
    <div class="row">
      <span class="lbl">Size</span>
      <span>{info.width} × {info.height}</span>
    </div>
    <div class="row">
      <span class="lbl">Padding</span>
      <span>{info.paddingTop} {info.paddingRight} {info.paddingBottom} {info.paddingLeft}</span>
    </div>
    <div class="row">
      <span class="lbl">Margin</span>
      <span>{info.marginTop} {info.marginRight} {info.marginBottom} {info.marginLeft}</span>
    </div>
    <div class="row">
      <span class="lbl">z-index</span>
      <span>{info.zIndex}</span>
    </div>
  </div>
{/if}

<style>
  .info-box {
    position: fixed;
    z-index: 8020;
    pointer-events: none;
    background: rgba(15, 23, 42, 0.92);
    border: 1px solid rgba(99, 102, 241, 0.4);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
    font-size: 0.72rem;
    color: #e2e8f0;
    min-width: 160px;
    max-width: 240px;
    backdrop-filter: blur(4px);
  }

  .tag {
    font-weight: 600;
    color: #93c5fd;
    margin-bottom: 0.15rem;
  }

  .cls {
    color: #86efac;
    margin-bottom: 0.3rem;
    word-break: break-all;
    font-size: 0.68rem;
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    line-height: 1.5;
  }

  .lbl {
    color: #94a3b8;
    flex-shrink: 0;
  }
</style>
