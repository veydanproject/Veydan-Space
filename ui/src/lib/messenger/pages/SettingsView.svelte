<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import IdentityCard from '../identity/IdentityCard.svelte';
  import OwnProfileCard from '../contacts/OwnProfileCard.svelte';
  import RelaysPanel from '../relays/RelaysPanel.svelte';
  import NetPanel from '../net/NetPanel.svelte';
  import CallNodesPanel from '../net/CallNodesPanel.svelte';
  import MediaServersPanel from '../media/MediaServersPanel.svelte';
  import NotificationsPanel from '../push/NotificationsPanel.svelte';
  import PrivacyPanel from '../privacy/PrivacyPanel.svelte';
  import CallsPanel from '../calls/CallsPanel.svelte';
  import DebugFeed from '../debug/DebugFeed.svelte';
  import { SETTINGS_TABS, settingsRequest, type SettingsTab } from './settingsTab.svelte';

  interface Props {
    /** The host screen already shows the title (phone). */
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  let tab = $state<SettingsTab>('profile');
  let chips = $state<HTMLElement | null>(null);

  // Asked for from elsewhere (a long press on a call button): that tab, its chip in view.
  $effect(() => {
    const want = settingsRequest.tab;
    if (!want) return;
    settingsRequest.tab = null;
    tab = want;
    void tick().then(() => chips?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ inline: 'nearest', block: 'nearest' }));
  });
  const s = $derived(messengerStore.status);

  let media = $state<HTMLElement | null>(null);

  // From the profile: an avatar needs a media server to be stored on.
  async function openMedia() {
    tab = 'network';
    await tick();
    media?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
</script>

<div class="settings">
  {#if compact}
    <!-- The phone: the sections as the shell's segment chips, kept in view
         while the section scrolls; a chip cut at the edge says the row scrolls. -->
    <div class="m-seg seg-sticky" role="tablist" bind:this={chips}>
      {#each SETTINGS_TABS as id}
        <button role="tab" class="m-seg-btn" class:active={tab === id} aria-selected={tab === id}
          onclick={(e) => { tab = id; (e.currentTarget as HTMLElement).scrollIntoView({ inline: 'nearest', block: 'nearest' }); }}>
          {$t(`msg_settings_tab_${id}` as 'msg_settings_tab_profile')}
        </button>
      {/each}
    </div>
  {:else}
    <h2>{$t('msg_settings_title_full')}</h2>
    <div class="tabs" role="tablist">
      {#each SETTINGS_TABS as id}
        <button role="tab" class="tab" class:active={tab === id} aria-selected={tab === id} onclick={() => (tab = id)}>
          {$t(`msg_settings_tab_${id}` as 'msg_settings_tab_profile')}
        </button>
      {/each}
    </div>
  {/if}

  {#if tab === 'profile'}
    <!-- No profile card for an identity that is gone (removed a moment ago) -->
    {#if messengerStore.identity}
      <OwnProfileCard onopenmedia={openMedia} />
      <IdentityCard identity={messengerStore.identity} />
    {/if}
  {:else if tab === 'network'}
    <NetPanel />
    <CallNodesPanel />
    <RelaysPanel />
    <div class="media-anchor" bind:this={media}><MediaServersPanel /></div>
  {:else if tab === 'notifications'}
    <NotificationsPanel />
  {:else if tab === 'privacy'}
    <PrivacyPanel />
  {:else if tab === 'calls'}
    <CallsPanel />
  {:else if s?.runtime}
    <div class="card status-card">
      <div class="card-title">{$t('msg_status_title')}</div>
      <dl class="status-grid">
        <dt>{$t('msg_status_version')}</dt><dd><code>{s.runtime.version}</code></dd>
        <dt>{$t('msg_status_schema')}</dt><dd><code>{s.runtime.schema_version}</code></dd>
        <dt>{$t('msg_status_data_dir')}</dt><dd><code>{s.runtime.data_dir}</code></dd>
        <dt>{$t('msg_status_secrets')}</dt>
        <dd>{s.runtime.secrets_unlocked ? $t('msg_status_secrets_unlocked') : $t('msg_status_secrets_locked')}</dd>
        <dt>{$t('msg_status_relays')}</dt>
        <dd>{$t('msg_status_relays_value', { connected: String(s.runtime.relays_connected), total: String(s.runtime.relays_total) })}{s.runtime.silent_mode ? ` · ${$t('msg_relays_silent')}` : ''}</dd>
        <dt>{$t('msg_status_session')}</dt>
        <dd>{s.runtime.session_active ? $t('msg_status_session_yes') : $t('msg_status_session_no')}</dd>
        <dt>{$t('msg_status_ingress')}</dt>
        <dd>{$t('msg_status_ingress_value', { received: String(s.runtime.ingress.received), dm: String(s.runtime.ingress.dm), dup: String(s.runtime.ingress.duplicates), ignored: String(s.runtime.ingress.ignored) })}</dd>
        <dt>{$t('msg_status_outbox')}</dt>
        <dd>{String(s.runtime.outbox_pending)}</dd>
      </dl>
      <button class="btn btn-ghost btn-sm" onclick={() => messengerStore.refresh()}>{$t('msg_status_refresh')}</button>
    </div>
    <DebugFeed />
  {/if}
</div>

<style>
  .settings { display: flex; flex-direction: column; gap: var(--sp-4); max-width: 680px; width: 100%; margin-inline: auto; }
  h2 { margin: 0; font-size: var(--fs-xl); font-weight: var(--fw-extrabold); letter-spacing: -0.4px; }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--border); overflow-x: auto; scrollbar-width: none; }
  .tab { white-space: nowrap; }
  .tab {
    border: none; background: none; font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-semibold);
    color: var(--text-2); padding: 8px 12px; cursor: pointer; border-bottom: 2px solid transparent; margin-bottom: -1px;
  }
  .tab:hover { color: var(--text); }
  .tab.active { color: var(--accent-text-2); border-bottom-color: var(--accent); }
  .settings :global(.card) { max-width: none; }
  .media-anchor { scroll-margin-top: var(--sp-12); }
  .status-card { display: flex; flex-direction: column; gap: var(--sp-3); align-items: flex-start; }
  .status-grid { display: grid; grid-template-columns: max-content 1fr; gap: var(--sp-2) var(--sp-4); margin: 0; font-size: var(--fs-sm); }
  .status-grid dt { color: var(--text-3); }
  /* Words wrap between words; only a path or a key may break anywhere. */
  .status-grid dd { margin: 0; min-width: 0; color: var(--text-body); }
  .status-grid code { overflow-wrap: anywhere; }
  /* Under the screen's bar, over the content that scrolls beneath it. */
  .seg-sticky {
    position: sticky; top: calc(-1 * var(--sp-3)); z-index: 1; margin: calc(-1 * var(--sp-3)) calc(-1 * var(--sp-3)) 0;
    padding: var(--sp-3) var(--sp-3) var(--sp-2); background: var(--bg);
  }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); background: var(--surface-2); padding: 2px 7px; border-radius: 6px; }
</style>
