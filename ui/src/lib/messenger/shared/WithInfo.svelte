<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A conversation and, when asked for, the panel about it beside it. Where
  there is no room for two columns the panel covers the conversation.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** The panel is shown. */
    open: boolean;
    chat: Snippet;
    info: Snippet;
  }
  let { open, chat, info }: Props = $props();
</script>

<div class="wrap" class:with-info={open}>
  <div class="main">{@render chat()}</div>
  {#if open}<aside class="side">{@render info()}</aside>{/if}
</div>

<style>
  .wrap { display: grid; grid-template-columns: minmax(0, 1fr); height: 100%; min-height: 0; position: relative; }
  .wrap.with-info { grid-template-columns: minmax(0, 1fr) 340px; }
  .main { min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .main > :global(*) { flex: 1; min-height: 0; }
  .side { min-height: 0; border-left: 1px solid var(--border); background: var(--surface); display: flex; flex-direction: column; }
  @media (max-width: 1100px) {
    .wrap.with-info { grid-template-columns: minmax(0, 1fr); }
    .side { position: absolute; inset: 0; border-left: none; z-index: 5; }
  }
</style>
