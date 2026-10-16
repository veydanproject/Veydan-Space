<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- What the cloud has to say over any page: the alerts of a recovery first, then the room that runs out. One column in one corner, so two of them never lie on each other; on a phone it stands above the bar of the navigation. -->
<script lang="ts">
  import { isMobile } from '$lib/core/platform';
  import QuotaBanner from '$lib/cloud/QuotaBanner.svelte';
  import RecoveryAlert from '$lib/cloud/RecoveryAlert.svelte';
</script>

<div class="banners" class:m={isMobile}>
  <RecoveryAlert />
  <QuotaBanner />
</div>

<style>
  .banners {
    position: fixed; right: var(--sp-4); bottom: var(--sp-4); z-index: var(--z-panel);
    width: min(440px, calc(100vw - 2 * var(--sp-4)));
    display: flex; flex-direction: column; gap: var(--sp-2); pointer-events: none;
    max-height: calc(100vh - 2 * var(--sp-4)); overflow-y: auto;
  }
  .banners.m {
    left: var(--sp-3); right: var(--sp-3); width: auto;
    bottom: calc(var(--nav-h, 64px) + var(--nav-gap, 0px) + var(--sab, 0px) + var(--sp-3));
    max-height: 70vh;
  }
  .banners > :global(*) { pointer-events: auto; }
  /* The scanner takes the whole screen and shows the camera through it: nothing of the app lies over that. */
  :global(html.scan-cover) .banners { visibility: hidden; }
</style>
