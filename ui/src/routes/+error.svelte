<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- An address without a page (a route of a module this product lacks): back to the start (platform-spec 11.2). -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';

  // The way out is shown only when the start screen is slow to come: an
  // address that goes on at once does not flash it.
  let late = $state(false);

  onMount(() => {
    goto('/', { replaceState: true });
    const timer = setTimeout(() => (late = true), 1500);
    return () => clearTimeout(timer);
  });
</script>

{#if late}
  <div class="lost">
    <p>{$t('screen_failed')}</p>
    <a href="/" data-sveltekit-replacestate>{$t('screen_to_start')}</a>
  </div>
{/if}

<style>
  .lost {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    padding: var(--sp-6) var(--sp-4);
    text-align: center;
    color: var(--text-2);
  }
  .lost p { margin: 0; }
  .lost a { color: var(--accent); font-weight: 600; }
</style>
