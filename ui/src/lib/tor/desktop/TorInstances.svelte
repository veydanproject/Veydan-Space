<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The state section of the Tor page: the instances that run and a way to start one by hand. -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { formatError } from '$lib/core/utils';
  import { api } from '$lib/tor/api';
  import { watchInstances } from '$lib/tor/instances';
  import { exitArgument, torErrorText } from '$lib/tor/settings';
  import type { InstanceInfo } from '$lib/tor/types';
  import TorInstanceRow from '$lib/tor/desktop/TorInstanceRow.svelte';

  let list = $state<InstanceInfo[]>([]);
  let exit = $state('');
  let starting = $state(false);
  let error = $state('');
  let stopWatching: (() => void) | null = null;

  onMount(() => {
    stopWatching = watchInstances((l) => (list = l));
  });
  onDestroy(() => stopWatching?.());

  async function start() {
    const { value, invalid } = exitArgument(exit);
    if (invalid.length) {
      error = $t('tor_err_countries', { list: invalid.join(', ') });
      return;
    }
    error = '';
    starting = true;
    try {
      // Returns at once; the row appears and fills by the event.
      await api.tor.start(value);
      exit = '';
    } catch (e) {
      error = torErrorText(formatError(e), (k) => $t(k));
    } finally {
      starting = false;
    }
  }
</script>

<div class="tor-instances">
  {#if list.length === 0}
    <p class="muted">{$t('tor_instances_empty')}</p>
  {:else}
    <div class="list">
      {#each list as info (info.key)}
        <TorInstanceRow {info} />
      {/each}
    </div>
  {/if}

  <form class="start" onsubmit={(e) => { e.preventDefault(); start(); }}>
    <div class="form-group">
      <label for="tor-start-exit">{$t('tor_start_label')}</label>
      <div class="start-row">
        <input id="tor-start-exit" type="text" bind:value={exit} placeholder={$t('tor_countries_placeholder')} autocomplete="off" spellcheck="false" />
        <button class="btn btn-primary btn-sm" type="submit" disabled={starting}>
          <Icon name="play" size={14} />
          {$t('tor_btn_start')}
        </button>
      </div>
      <span class="muted small">{$t('tor_start_hint')}</span>
    </div>
    {#if error}<div class="error-msg">{error}</div>{/if}
  </form>
</div>

<style>
  .tor-instances { display: flex; flex-direction: column; gap: var(--sp-4); }
  .list { display: flex; flex-direction: column; gap: var(--sp-3); }
  .start { display: flex; flex-direction: column; gap: var(--sp-2); }
  .start-row { display: flex; gap: var(--sp-2); align-items: center; }
  .small { font-size: var(--fs-sm); }
  .error-msg { font-size: var(--fs-sm); color: var(--danger-text); }
</style>
