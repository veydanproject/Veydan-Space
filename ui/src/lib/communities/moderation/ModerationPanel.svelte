<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/moderation/ModerationPanel.svelte at 8ce09c39, then changed for channels. -->

<!--
  The moderation of a channel, a mode of its panel (on a phone the panel is
  the screen, so this is too): the requests to be let in, the queue of what
  waits for a check, the log of decisions, and for a manager the rules —
  who posts, the comments, the check, the hours.
-->
<script lang="ts" module>
  export type ModerationTab = 'requests' | 'levels' | 'queue' | 'log' | 'settings';
</script>

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { PanelHead } from '$lib/messenger/seams/shared';
  import RequestsTab from './RequestsTab.svelte';
  import QueueTab from './QueueTab.svelte';
  import LogTab from './LogTab.svelte';
  import SettingsTab from './SettingsTab.svelte';
  import LevelRequestsTab from '../levels/LevelRequestsTab.svelte';
  import TicketsLine from './TicketsLine.svelte';
  import { showsTickets } from '../tickets';
  import { levelsOf } from '../levels/sections';
  import { channelStore } from '../channelStore.svelte';
  import { answersRequests, isManager, isModerator } from '../permissions';
  import type { ChannelDetail } from '../api';

  interface Props {
    detail: ChannelDetail;
    /** The tab shown; the one who opens the panel may ask for one. */
    tab?: ModerationTab;
    onback: () => void;
  }
  let { detail, tab = $bindable('queue'), onback }: Props = $props();

  const cid = $derived(detail.channel.id);

  const tabs = $derived.by<ModerationTab[]>(() => {
    const list: ModerationTab[] = [];
    // Requests come to a channel that takes them, or are left from before its way in changed.
    if (answersRequests(detail) && (detail.channel.join === 'request' || (channelStore.requests[cid] ?? 0) > 0)) list.push('requests');
    // Requests for a level come to a channel with levels to give, or are left from before the last one went.
    if (isModerator(detail) && (levelsOf(detail).levels.length > 1 || (channelStore.levelRequests[cid] ?? 0) > 0)) list.push('levels');
    if (isModerator(detail)) list.push('queue', 'log');
    if (isManager(detail)) list.push('settings');
    return list;
  });
  // A moderator who lost the role, or a manager demoted, stays on what is still theirs.
  $effect(() => { if (tabs.length && !tabs.includes(tab)) tab = tabs[0]; });

  /** The count a tab carries: what waits there. */
  const count = (v: ModerationTab): number =>
    v === 'requests' ? (channelStore.requests[cid] ?? 0) : v === 'levels' ? (channelStore.levelRequests[cid] ?? 0) : v === 'queue' ? (channelStore.queue[cid] ?? 0) : 0;
</script>

<PanelHead title={$t('chan_mod_title')} {onback} />

<div class="scroll">
  {#if showsTickets(detail)}<TicketsLine {detail} />{/if}
  {#if tabs.length > 1}
    <div class="seg tabs" role="tablist" aria-label={$t('chan_mod_title')}>
      {#each tabs as v (v)}
        {@const label = $t(`chan_mod_tab_${v}` as 'chan_mod_tab_queue')}
        <!-- Every tab says its word; when they do not fit the panel's width they scroll sideways. -->
        <button class="seg-btn" class:active={tab === v} role="tab" aria-selected={tab === v} onclick={() => (tab = v)}>
          {#if v === 'settings'}<Icon name="settings" size={14} />{/if}<span class="tab-name">{label}</span>
          {#if count(v)}<span class="tab-count">{count(v) > 99 ? '99+' : count(v)}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  {#if tab === 'requests'}<RequestsTab {detail} />
  {:else if tab === 'levels'}<LevelRequestsTab {detail} />
  {:else if tab === 'queue'}<QueueTab {detail} />
  {:else if tab === 'log'}<LogTab {detail} />
  {:else}<SettingsTab {detail} />{/if}
</div>

<style>
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  /* The tabs take the width when they fit and scroll sideways when a language makes them longer. */
  .tabs { align-self: stretch; flex-shrink: 0; overflow-x: auto; scrollbar-width: none; }
  .tabs::-webkit-scrollbar { display: none; }
  .tabs .seg-btn { flex: 1 0 auto; justify-content: center; gap: 5px; padding: 0 var(--sp-2); }
  .seg-btn.active .tab-count { background: var(--accent); color: #fff; }
</style>
