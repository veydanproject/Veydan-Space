<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A box of a channel's info (its contents, its pages, a reader's blocks):
  rows with a hairline between two (`rows`; `fromText` starts the line at
  the text of a row with a tile, as in the contents), or a card of text and
  buttons (`text`; `tight` for the keeper's). Its own class, not the global
  `.card`, whose shadow and motion are a pane's.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  interface Props extends HTMLAttributes<HTMLElement> {
    tag?: 'div' | 'ul' | 'section';
    kind?: 'rows' | 'text';
    fromText?: boolean;
    tight?: boolean;
    /** Its content at the start instead of stretched (a row of chips). */
    start?: boolean;
    children: Snippet;
  }
  let { tag = 'div', kind = 'rows', fromText = false, tight = false, start = false, children, ...rest }: Props = $props();
</script>

<svelte:element this={tag} {...rest} class="box {kind}" class:from-text={fromText} class:tight class:start>{@render children()}</svelte:element>

<style>
  .box { --line-left: var(--sp-3); display: flex; flex-direction: column; border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  /* The row's padding, its tile (36), the gap. */
  .from-text { --line-left: calc(var(--sp-3) * 2 + 36px); }
  .rows { list-style: none; margin: 0; padding: var(--sp-1); }
  .rows > :global(*) { position: relative; }
  .rows > :global(*:not(:first-child))::before {
    content: ''; position: absolute; top: 0; left: var(--line-left); right: var(--sp-3); height: 1px; background: var(--border-2);
  }
  .text { gap: var(--sp-3); padding: var(--sp-3); }
  .text.tight { gap: var(--sp-2); }
  .start { align-items: flex-start; }
</style>
