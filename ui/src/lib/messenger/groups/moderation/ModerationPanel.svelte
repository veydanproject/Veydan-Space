<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The moderation of a group, a mode of its panel (on a phone the panel is
  the screen, so this is too): for a manager the requests to join, the
  queue of what waits for a check, the log of decisions, and who may write
  and when. A core without moderation has the requests alone.
-->
<script lang="ts" module>
  export type ModerationTab = 'requests' | 'queue' | 'log' | 'settings';
</script>

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import PanelHead from '../../shared/PanelHead.svelte';
  import RequestsTab from './RequestsTab.svelte';
  import QueueTab from './QueueTab.svelte';
  import LogTab from './LogTab.svelte';
  import SettingsTab from './SettingsTab.svelte';
  import { answersRequests, hasModeration, isManager, isModerator } from '../permissions';
  import type { MessengerGroup } from '../../api';

  interface Props {
    group: MessengerGroup;
    /** The tab shown; the one who opens the panel may ask for one. */
    tab?: ModerationTab;
    onback: () => void;
  }
  let { group, tab = $bindable('queue'), onback }: Props = $props();

  const tabs = $derived.by<ModerationTab[]>(() => {
    const list: ModerationTab[] = [];
    // Letting people in is a manager's act: a moderator does not see the requests.
    if (answersRequests(group)) list.push('requests');
    if (isModerator(group) && hasModeration(group)) list.push('queue', 'log');
    if (isManager(group) && hasModeration(group)) list.push('settings');
    return list;
  });
  // A moderator who lost the role, or a manager demoted, stays on what is still theirs.
  $effect(() => { if (tabs.length && !tabs.includes(tab)) tab = tabs[0]; });

  /** The count a tab carries: what waits there. */
  const count = (v: ModerationTab): number => (v === 'requests' ? group.requests.length : v === 'queue' ? (group.queue ?? 0) : 0);
</script>

<PanelHead title={$t('msg_mod_title')} {onback} />

<div class="scroll">
  {#if tabs.length > 1}
    <div class="seg tabs" role="tablist" aria-label={$t('msg_mod_title')}>
      {#each tabs as v (v)}
        {@const label = $t(`msg_mod_tab_${v}` as 'msg_mod_tab_queue')}
        <!-- The settings are a gear: four words and two counts do not fit a panel's width. -->
        <button class="seg-btn" class:active={tab === v} class:glyph={v === 'settings'} role="tab" aria-selected={tab === v} onclick={() => (tab = v)}
          title={v === 'settings' ? label : undefined} aria-label={v === 'settings' ? label : undefined}>
          {#if v === 'settings'}<Icon name="settings" size={15} />{:else}<span class="tab-name">{label}</span>{/if}
          {#if count(v)}<span class="tab-count">{count(v) > 99 ? '99+' : count(v)}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  {#if tab === 'requests'}<RequestsTab {group} />
  {:else if tab === 'queue'}<QueueTab {group} />
  {:else if tab === 'log'}<LogTab {group} />
  {:else}<SettingsTab {group} />{/if}
</div>

<style>
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  /* The tabs take the width when they fit and scroll sideways when a language makes them longer. */
  .tabs { align-self: stretch; flex-shrink: 0; overflow-x: auto; scrollbar-width: none; }
  .tabs::-webkit-scrollbar { display: none; }
  .tabs .seg-btn { flex: 1 0 auto; justify-content: center; gap: 5px; padding: 0 var(--sp-2); }
  .tabs .seg-btn.glyph { flex: 0 0 auto; padding: 0 var(--sp-3); }
  .seg-btn.active .tab-count { background: var(--accent); color: #fff; }
</style>
