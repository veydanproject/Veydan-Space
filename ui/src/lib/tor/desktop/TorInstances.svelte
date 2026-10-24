<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The tab «Состояние» of the Tor section: a card a running instance, or the word that nothing runs, and a card to start one by hand. -->
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
  import TorNote from '$lib/tor/desktop/TorNote.svelte';

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
    <div class="card empty">
      <span class="tile" aria-hidden="true"><Icon name="globe" size={20} /></span>
      <p>{$t('tor_instances_empty')}</p>
    </div>
  {:else}
    {#each list as info (info.key)}
      <TorInstanceRow {info} />
    {/each}
  {/if}

  <form class="card start" onsubmit={(e) => { e.preventDefault(); start(); }}>
    <label for="tor-start-exit">{$t('tor_start_label')}</label>
    <div class="start-row">
      <input id="tor-start-exit" type="text" bind:value={exit} placeholder={$t('tor_countries_placeholder')} autocomplete="off" spellcheck="false" aria-describedby="tor-start-hint" />
      <button class="btn btn-primary" type="submit" disabled={starting}>
        <Icon name="play" size={18} />
        {$t('tor_btn_start')}
      </button>
    </div>
    <span class="hint" id="tor-start-hint">{$t('tor_start_hint')}</span>
    {#if error}<TorNote tone="danger">{error}</TorNote>{/if}
  </form>
</div>

<style>
  .tor-instances { display: flex; flex-direction: column; gap: var(--sp-3); }
  .empty { display: flex; align-items: flex-start; gap: var(--sp-4); padding: var(--sp-5); }
  .empty p { margin: 0; font-size: var(--fs-base); line-height: 1.55; color: var(--text-2); }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: var(--control-h); height: var(--control-h); flex-shrink: 0;
    border-radius: var(--radius); background: var(--accent-tint); color: var(--accent-text);
  }
  .start { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-4) var(--sp-5); }
  .start label { font-size: var(--fs-base); font-weight: var(--fw-semibold); }
  .start-row { display: flex; gap: var(--sp-2); align-items: center; }
  .start-row input { flex: 1; min-width: 0; }
  .hint { font-size: var(--fs-sm); line-height: 1.45; color: var(--text-2); }
</style>
