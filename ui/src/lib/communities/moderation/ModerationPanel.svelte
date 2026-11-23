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
  import { scrollTabs } from '$lib/core/ui/scrollTabs';
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

  // A narrow panel clips the row of tabs: it scrolls sideways (the wheel of a mouse too), the chosen tab
  // stays in view, and a clipped edge fades so the row reads as going on (as the tabs of the account).
  // The fades follow the row's room: other tabs or their counts (a count widens its tab), the panel's width.
  const widths = $derived(tabs.map((v) => `${v}:${count(v)}`).join());
</script>

<PanelHead title={$t('chan_mod_title')} {onback} />

<div class="scroll">
  {#if showsTickets(detail)}<TicketsLine {detail} />{/if}
  {#if tabs.length > 1}
    <div class="tab-bar tabs" role="tablist" aria-label={$t('chan_mod_title')} use:scrollTabs={{ selected: tab, watch: widths, wheel: true, fadeStart: true }}>
      {#each tabs as v (v)}
        {@const label = $t(`chan_mod_tab_${v}` as 'chan_mod_tab_queue')}
        <!-- Every tab says its word; when they do not fit the panel's width they scroll sideways. -->
        <button class="tab" class:active={tab === v} role="tab" aria-selected={tab === v} onclick={() => (tab = v)}>
          {#if v === 'settings'}<Icon name="settings" size={14} />{/if}<span class="tab-name">{label}</span>
          {#if count(v)}<span class="count-pill sm quiet">{count(v) > 99 ? '99+' : count(v)}</span>{/if}
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
  /* The tabs are words on the panel's background: no track, the chosen one bold with a short mark; they scroll sideways
     when a language makes them longer and a clipped edge fades. */
  /* The tabs are the page's .tab-bar; here only the layout: the panel's width, scrolled sideways when a language makes
     them longer, a clipped edge fades. */
  .tabs { align-self: stretch; overflow-x: auto; scrollbar-width: none; }
  .tabs::-webkit-scrollbar { display: none; }
  .tabs .tab { flex: 0 0 auto; justify-content: center; }
  /* The keyboard's ring inside the tab: the row clips what sticks out. */
  .tabs .tab:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
  /* A clipped edge fades, only while there is more of the row on that side (scrollTabs marks it). */
  .tabs:global(.fade-end) { -webkit-mask-image: linear-gradient(to right, black 85%, transparent); mask-image: linear-gradient(to right, black 85%, transparent); }
  .tabs:global(.fade-start) { -webkit-mask-image: linear-gradient(to left, black 85%, transparent); mask-image: linear-gradient(to left, black 85%, transparent); }
  .tabs:global(.fade-start.fade-end) {
    -webkit-mask-image: linear-gradient(to right, transparent, black 15%, black 85%, transparent);
    mask-image: linear-gradient(to right, transparent, black 15%, black 85%, transparent);
  }
  .tabs .count-pill.quiet { background: var(--surface-3); color: var(--text-2); }
</style>
