<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A conversation and, when asked for, the panel about it beside it. Where
  there is no room for two columns the panel covers the conversation from
  the right; on a phone it covers it whole.

  On a computer the gap before the panel changes its width (PaneResizer):
  320 px up to 720 and never more than 60 % of the pane, 340 by default.
  The width is kept by the runtime under `prefKey` and is the same on every
  computer of mine (uiPrefs); a phone keeps its own layout.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import PaneResizer from '$lib/core/ui/PaneResizer.svelte';
  import { clampWidth, sideMax } from '$lib/core/ui/paneWidth';
  import { onPhone } from './phone';
  import { uiPrefs } from './uiPrefs.svelte';

  interface Props {
    /** The panel is shown. */
    open: boolean;
    chat: Snippet;
    info: Snippet;
    /** Under which key the width is kept: `chats.side` (a chat), `channels.side` (a channel). */
    prefKey?: string;
  }
  let { open, chat, info, prefKey = 'chats.side' }: Props = $props();

  const MIN = 320;
  const CAP = 720;
  const DEFAULT = 340;

  const phone = onPhone();
  let paneWidth = $state(0);
  /** The width while it is dragged, before it is kept. */
  let live = $state<number | null>(null);
  const max = $derived(sideMax(paneWidth, MIN, CAP));
  // A phone keeps its own layout: the width of my computers is not for it.
  const width = $derived(phone ? DEFAULT : clampWidth(live ?? uiPrefs.width(prefKey) ?? DEFAULT, MIN, max));

  function commit(px: number | null) {
    live = null;
    uiPrefs.set(prefKey, px);
  }
</script>

<div class="wrap" class:with-info={open} class:phone bind:clientWidth={paneWidth} style:--side-w="{width}px">
  <div class="main">{@render chat()}</div>
  {#if open}
    {#if !phone}
      <div class="edge">
        <PaneResizer value={width} min={MIN} {max} defaultValue={DEFAULT} oninput={(px) => (live = px)} oncommit={commit} />
      </div>
    {/if}
    <aside class="side">{@render info()}</aside>
  {/if}
</div>

<style>
  .wrap { display: grid; grid-template-columns: minmax(0, 1fr); height: 100%; min-height: 0; position: relative; }
  .wrap.with-info { grid-template-columns: minmax(0, 1fr) 12px var(--side-w, 340px); }
  .wrap.with-info.phone { grid-template-columns: minmax(0, 1fr) var(--side-w, 340px); }
  .main { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .main > :global(*) { flex: 1; min-height: 0; }
  .edge { min-height: 0; display: flex; }
  .side { min-width: 0; min-height: 0; border-left: 1px solid var(--border); background: var(--surface); display: flex; flex-direction: column; }
  @media (max-width: 1100px) {
    .wrap.with-info, .wrap.with-info.phone { grid-template-columns: minmax(0, 1fr); }
    /* No room for two columns: the panel lies over the conversation, as wide as on a wide window. */
    .side { position: absolute; top: 0; bottom: 0; right: 0; width: min(var(--side-w, 340px), 100%); z-index: 5; box-shadow: -12px 0 32px rgba(0, 0, 0, 0.18); }
    .edge { position: absolute; top: 0; bottom: 0; right: min(var(--side-w, 340px), 100%); z-index: 6; }
    .wrap.phone .side { left: 0; width: auto; border-left: none; box-shadow: none; }
  }
</style>
