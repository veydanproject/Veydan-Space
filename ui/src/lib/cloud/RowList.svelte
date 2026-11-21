<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A group of rows of the page of the cloud under one outline: a card on a computer, a grouped list on a phone. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { isMobile } from '$lib/core/platform';

  let { label, id, children }: { label?: string; id?: string; children: Snippet } = $props();
</script>

<section class="group" class:labelled={!!label} class:m={isMobile} {id}>
  {#if label}<h2 class="section-label">{label}</h2>{/if}
  <div class={isMobile ? 'm-list' : 'card rows'}>{@render children()}</div>
</section>

<style>
  .group { display: flex; flex-direction: column; }
  h2 { margin: 0; }
  /* The common .section-label: the space above it and to the card is the group's (on a phone 16 above, as between
     the groups of a phone's page). */
  .labelled { gap: var(--sp-2); padding-top: var(--sp-3); }
  .labelled.m { padding-top: var(--sp-4); }
  /* Not clipped: the keyboard's ring of a row stands outside it, as a button's; the first and the last row
     take the card's corners for their hover instead. */
  .rows { padding: 0; }
  .rows > :global(:first-child) { border-top-left-radius: inherit; border-top-right-radius: inherit; }
  .rows > :global(:last-child) { border-bottom-left-radius: inherit; border-bottom-right-radius: inherit; }
</style>
