<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- What the cloud has to say over any page: the alerts of a recovery first, then the room that runs out. One column in one corner, so two of them never lie on each other; on a phone it is the top of the page, under its head, in its flow (design 18): it scrolls with the page and lies on none of its buttons. -->
<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import QuotaBanner from '$lib/cloud/QuotaBanner.svelte';
  import RecoveryAlert from '$lib/cloud/RecoveryAlert.svelte';
  import { zone } from '$lib/core/ui/zone';
  import { pageTop } from '$lib/core/mobile/page-top';
</script>

<!-- On a computer the toast stands over this column (--zone-cloud), not on its buttons; the column leaves the toast its room (--zone-toast), so the toast's top stays in the window. -->
{#if isMobile}
  <div class="banners m" use:pageTop>
    <RecoveryAlert />
    <QuotaBanner />
  </div>
{:else}
  <div class="banners" use:zone={'--zone-cloud'}>
    <RecoveryAlert />
    <QuotaBanner />
  </div>
{/if}

<style>
  .banners {
    position: fixed; right: var(--sp-4); bottom: var(--sp-4); z-index: var(--z-panel);
    width: min(440px, calc(100vw - 2 * var(--sp-4)));
    display: flex; flex-direction: column; gap: var(--sp-2); pointer-events: none;
    max-height: calc(100vh - 2 * var(--sp-4) - var(--zone-toast, 0px)); overflow-y: auto;
  }
  .banners > :global(*) { pointer-events: auto; }
  /* Its height is what it holds; empty, it takes no room of the page. */
  .banners.m { position: static; width: auto; max-height: none; overflow: visible; pointer-events: auto; }
  .banners.m:not(:empty) { margin-bottom: var(--sp-3); }
  /* The scanner takes the whole screen and shows the camera through it: nothing of the app lies over that. */
  :global(html.scan-cover) .banners { visibility: hidden; opacity: 0; }
</style>
