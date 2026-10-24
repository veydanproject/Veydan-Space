<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The section Tor of the settings: what is installed and how many instances
  run in its head, then three tabs — the instances (Состояние), the install
  (Установка) and the settings of the daemon (Настройки Tor). It is the whole
  of Tor; the page /tor leads here.
-->
<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { watchInstances } from '$lib/tor/instances';
  import { runningCount } from '$lib/tor/settings';
  import { versionLine } from '$lib/tor/format';
  import type { TorStatus } from '$lib/tor/types';
  import TorInstall from '$lib/tor/desktop/TorInstall.svelte';
  import TorInstances from '$lib/tor/desktop/TorInstances.svelte';
  import TorSettingsForm from '$lib/tor/desktop/TorSettingsForm.svelte';

  type Tab = 'state' | 'install' | 'settings';
  const TABS: { id: Tab; label: TranslationKey }[] = [
    { id: 'state', label: 'tor_page_section_state' },
    { id: 'install', label: 'tor_page_section_install' },
    { id: 'settings', label: 'tor_btn_settings' },
  ];

  let running = $state(0);
  let status = $state<TorStatus | null>(null);
  let tab = $state<Tab>('state');
  let tabsEl = $state<HTMLElement | null>(null);
  /** The tab of the settings has been opened once. */
  let formSeen = $state(false);
  $effect(() => {
    if (tab === 'settings') formSeen = true;
  });
  let stopWatching: (() => void) | null = null;

  onMount(() => {
    stopWatching = watchInstances((list) => (running = runningCount(list)));
  });
  onDestroy(() => stopWatching?.());

  /** The arrows move along the tabs (the keyboard's way through a tablist). */
  function tabKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const i = TABS.findIndex((x) => x.id === tab);
    tab = TABS[(i + (e.key === 'ArrowRight' ? 1 : TABS.length - 1)) % TABS.length].id;
    void tick().then(() => tabsEl?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus());
  }
</script>

<div class="tor">
  <div class="head">
    <span class="tile" aria-hidden="true"><Icon name="globe" size={22} /></span>
    <span class="who">
      <strong>{$t('settings_tor_title')}</strong>
      {#if (status?.installed && (status.version || status.tor_version)) || running > 0}
        <span class="sub">
          {#if status?.installed && (status.version || status.tor_version)}<span class="mono">{versionLine(status.version, status.tor_version)}</span>{/if}
          {#if status?.installed && (status.version || status.tor_version) && running > 0} · {/if}
          {#if running > 0}{$t('tor_card_running', { count: String(running) })}{/if}
        </span>
      {/if}
    </span>
    {#if status}
      <span class="badge" class:badge-ok={status.installed} class:badge-warn={!status.installed}>
        {status.installed ? $t('tor_installed') : $t('tor_not_installed')}
      </span>
      {#if status.installed && status.source === 'manual'}
        <span class="badge badge-warn">{$t('tor_installed_by_hand')}</span>
      {/if}
    {/if}
  </div>

  <div class="tabs" role="tablist" aria-label={$t('settings_tor_title')} tabindex="-1" bind:this={tabsEl} onkeydown={tabKey}>
    {#each TABS as x (x.id)}
      <button
        type="button"
        role="tab"
        class="ctab"
        id="tor-tab-{x.id}"
        aria-selected={tab === x.id}
        aria-controls="tor-panel-{x.id}"
        tabindex={tab === x.id ? 0 : -1}
        onclick={() => (tab = x.id)}
      >{$t(x.label)}</button>
    {/each}
  </div>

  <!-- Every panel stays mounted (a running download, an unsaved form keep their state); only the chosen one shows. -->
  <div class="panel" role="tabpanel" id="tor-panel-state" aria-labelledby="tor-tab-state" hidden={tab !== 'state'}><TorInstances /></div>
  <div class="panel" role="tabpanel" id="tor-panel-install" aria-labelledby="tor-tab-install" hidden={tab !== 'install'}><TorInstall onstatus={(s) => (status = s)} /></div>
  <!-- The form mounts the first time its tab opens, and stays: a select bound while its panel is hidden shows its first option, not the stored one. -->
  <div class="panel" role="tabpanel" id="tor-panel-settings" aria-labelledby="tor-tab-settings" hidden={tab !== 'settings'}>{#if formSeen}<TorSettingsForm />{/if}</div>
</div>

<style>
  .tor { display: flex; flex-direction: column; gap: var(--sp-4); }
  .head { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: var(--control-h-lg); height: var(--control-h-lg); flex-shrink: 0;
    border-radius: var(--radius-md); background: var(--accent-tint); color: var(--accent-text);
  }
  .who { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .who strong { font-size: var(--fs-lg); font-weight: var(--fw-bold); line-height: 1.3; }
  .sub { font-size: var(--fs-sm); color: var(--text-2); }
  .tabs { display: flex; gap: var(--sp-1); border-bottom: 1px solid var(--divider); overflow-x: auto; scrollbar-width: none; }
  .ctab {
    flex-shrink: 0; min-height: var(--control-h-lg); padding: 0 var(--sp-3); margin-bottom: -1px;
    border: 0; border-bottom: 2px solid transparent; border-radius: 0; background: none;
    color: var(--text-2); font: inherit; font-size: var(--fs-base); font-weight: var(--fw-semibold); white-space: nowrap; cursor: pointer;
    transition: color var(--dur-fast), border-color var(--dur-fast);
  }
  .ctab:hover { color: var(--text); }
  .ctab[aria-selected='true'] { color: var(--accent-text); border-bottom-color: var(--accent); }
  .panel[hidden] { display: none; }
</style>
