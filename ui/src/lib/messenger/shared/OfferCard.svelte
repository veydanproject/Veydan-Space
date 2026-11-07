<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  An offer over the list of chats (a bridge, the notifications): one shape for
  all of them. The tile 36 on the left, its top on the title's line; on the
  right, a column 12 apart: the title, the phrase, the buttons (the verb first),
  what it costs smaller under them. One at a time (MobileChats).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Tile from '$lib/core/ui/Tile.svelte';

  let { icon, title, children, actions, note }: {
    icon: string; title: string; children: Snippet; actions: Snippet; note?: Snippet;
  } = $props();
</script>

<div class="offer">
  <Tile {icon} tone="accent" />
  <div class="col">
    <strong class="title">{title}</strong>
    <div class="text">{@render children()}</div>
    <div class="actions">{@render actions()}</div>
    {#if note}<div class="note">{@render note()}</div>{/if}
  </div>
</div>

<style>
  .offer {
    margin: var(--sp-2) var(--sp-3); padding: var(--sp-4); flex-shrink: 0;
    display: flex; align-items: flex-start; gap: var(--sp-3);
    border: 1px solid var(--accent-tint-border); border-radius: var(--radius-card); background: var(--accent-tint);
  }
  .col { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-3); }
  .title { font-size: var(--fs-body); font-weight: var(--fw-bold); line-height: var(--lh-snug); }
  .text { font-size: var(--fs-sm); color: var(--text-body); line-height: var(--lh-snug); }
  .text :global(p) { margin: 0; }
  .note { font-size: var(--fs-xs); color: var(--text-2); line-height: var(--lh-snug); }
  .note :global(p) { margin: 0; }
  /* Side by side, the verb the wider; a long word wraps them into two lines rather than clipping. */
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .actions :global(.btn) { flex: 1 1 auto; min-height: var(--control-h); justify-content: center; font-size: var(--fs-sm); }
  .actions :global(.btn-primary) { flex-grow: 1.4; }
</style>
