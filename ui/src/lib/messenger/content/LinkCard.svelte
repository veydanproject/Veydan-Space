<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The card of one link: where a link outside leads, what a link inside
  leads to, or a wait while the runtime is asked. The same card under a
  message and in the list of what a chat has shared.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { cardOf } from './cards';
  import ExternalCard from './cards/ExternalCard.svelte';
  import { linkStore } from './linkStore.svelte';
  import type { LinkSegment } from './types';

  interface Props { link: LinkSegment }
  let { link: l }: Props = $props();

  const view = $derived(l.type === 'internal' ? linkStore.view(l.link) : null);
</script>

<div class="link" class:pending={l.type === 'internal' && !view}>
  {#if l.type === 'external'}
    <ExternalCard url={l.url} />
  {:else if view}
    {@const Card = cardOf(view)}
    <Card {view} text={l.text} />
  {:else}
    <span class="wait"><Icon name="loader" size={14} />{l.text}</span>
  {/if}
</div>

<style>
  .link {
    padding: var(--sp-3); border-radius: var(--radius-md); border: 1px solid var(--border);
    background: var(--surface); color: var(--text); min-width: 0;
  }
  .link.pending { padding: var(--sp-2) var(--sp-3); }
  .wait { display: flex; align-items: center; gap: 6px; font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wait :global(svg) { flex-shrink: 0; animation: spin 1.1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
