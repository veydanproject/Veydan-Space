<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The Tor card of the settings page: the install block, a line of state and the way to the Tor page. -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { watchInstances } from '$lib/tor/instances';
  import { runningCount } from '$lib/tor/settings';
  import TorInstall from '$lib/tor/desktop/TorInstall.svelte';

  let running = $state(0);
  let stopWatching: (() => void) | null = null;

  onMount(() => {
    stopWatching = watchInstances((list) => (running = runningCount(list)));
  });
  onDestroy(() => stopWatching?.());
</script>

<TorInstall />

{#if running > 0}
  <p class="muted state-line">{$t('tor_card_running', { count: String(running) })}</p>
{/if}

<div class="btn-row">
  <button class="btn btn-ghost btn-sm" onclick={() => goto('/tor')}>
    <Icon name="settings" size={14} />
    {$t('tor_btn_settings')}
  </button>
</div>

<style>
  .state-line { font-size: var(--fs-sm); }
</style>
