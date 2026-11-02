<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Tabs from '$lib/core/ui/Tabs.svelte';
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
  import { SETTINGS_TABS, settingsRequest, settingsUi, type SettingsTab } from './settingsTab.svelte';

  interface Props {
    /** The host screen already shows the title (phone). */
    compact?: boolean;
    /** The host shows the tabs itself (the nav of the core's Settings): no row of tabs here. */
    hideTabs?: boolean;
  }
  let { compact = false, hideTabs = false }: Props = $props();

  // The shown section is shared with the desk's nav (MessengerPage).
  let tab = $derived<SettingsTab>(settingsUi.tab);
  const pick = (id: SettingsTab) => (settingsUi.tab = id);
  const tabs = $derived(SETTINGS_TABS.map((id) => ({ id, label: $t(`msg_settings_tab_${id}` as 'msg_settings_tab_profile') })));

  // Asked for from elsewhere (a long press on a call button): that tab, kept in view by the row.
  $effect(() => {
    const want = settingsRequest.tab;
    if (!want) return;
    settingsRequest.tab = null;
    pick(want);
  });
  const s = $derived(messengerStore.status);

  let media = $state<HTMLElement | null>(null);

  // From the profile: an avatar needs a media server to be stored on. The tab may be another view than this one
  // (the core's Settings mounts one per section), so the scroll waits for the element of whichever shows it.
  function openMedia() {
    settingsUi.focusMedia = true;
    pick('network');
  }
  $effect(() => {
    if (!settingsUi.focusMedia || tab !== 'network' || !media) return;
    settingsUi.focusMedia = false;
    void tick().then(() => media?.scrollIntoView({ behavior: 'smooth', block: 'start' }));
  });
</script>

<div class="settings" class:compact>
  {#if hideTabs}
    <!-- The tabs are the nav of the host. -->
  {:else if compact}
    <!-- The phone: the sections as the tabs of «Аккаунт», above the section that scrolls under them. -->
    <div class="tabs-fixed">
      <Tabs {tabs} selected={tab} onpick={pick} label={$t('msg_settings_title_full')} phone />
    </div>
  {:else}
    <!-- A narrow window, where the column of the nav is not shown: the sections as tabs. -->
    <div class="narrow-only">
      <Tabs {tabs} selected={tab} onpick={pick} label={$t('msg_settings_title_full')} />
    </div>
  {/if}

  <div class="sbody">
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
      <button class="btn btn-ghost" onclick={() => messengerStore.refresh()}>{$t('msg_status_refresh')}</button>
    </div>
    <DebugFeed />
  {/if}
  </div>
</div>

<style>
  .settings { display: flex; flex-direction: column; gap: var(--sp-4); max-width: 820px; width: 100%; }
  @media (min-width: 861px) { .narrow-only { display: none; } }
  /* Every section's cards alike: the glass card of base.css (radius of a card), padding 16/20, a title of the card's own
     size without an icon. */
  .settings :global(.card) { max-width: none; padding: var(--sp-4) var(--sp-5); box-shadow: none; }
  .media-anchor { scroll-margin-top: var(--sp-12); }
  .status-card { display: flex; flex-direction: column; gap: var(--sp-3); align-items: flex-start; }
  .status-grid { display: grid; grid-template-columns: max-content 1fr; gap: var(--sp-2) var(--sp-4); margin: 0; font-size: var(--fs-sm); }
  .status-grid dt { color: var(--text-3); }
  /* Words wrap between words; only a path or a key may break anywhere. */
  .status-grid dd { margin: 0; min-width: 0; color: var(--text-body); }
  .compact .status-grid code { display: block; overflow-wrap: anywhere; }
  .sbody { display: flex; flex-direction: column; gap: var(--sp-4); }
  /* The phone: the tabs stay above the section that scrolls; the section dissolves at their lower edge. The screen's frame
     does not scroll here (MobileSettings), the section does: its padding and its cards are the frame's own. */
  .settings.compact { flex: 1; min-height: 0; gap: 0; }
  .tabs-fixed { flex-shrink: 0; padding: var(--sp-3) var(--sp-3) 0; --tabs-bleed: var(--sp-3); }
  .compact .sbody {
    flex: 1; min-height: 0; overflow-y: auto; -webkit-overflow-scrolling: touch; gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-3) calc(var(--sp-5) + var(--sab, 0px));
    -webkit-mask-image: linear-gradient(to bottom, transparent, black var(--sp-5)); mask-image: linear-gradient(to bottom, transparent, black var(--sp-5));
  }
  .compact .sbody :global(.card) {
    max-width: none; width: 100%;
    background: var(--pane-strong); border-color: var(--m-card-border, var(--border));
    box-shadow: var(--m-card-shadow, none);
  }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); background: var(--surface-3); padding: 2px 7px; border-radius: var(--radius-badge); }
</style>
