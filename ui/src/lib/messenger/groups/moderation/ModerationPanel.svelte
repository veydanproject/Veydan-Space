<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The moderation of a group, a mode of its panel (on a phone the panel is
  the screen, so this is too): the queue of what waits for a check, the log
  of decisions, and, for a manager, who may write and when.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import PanelHead from '../../shared/PanelHead.svelte';
  import QueueTab from './QueueTab.svelte';
  import LogTab from './LogTab.svelte';
  import SettingsTab from './SettingsTab.svelte';
  import { isManager } from '../permissions';
  import type { MessengerGroup } from '../../api';

  interface Props {
    group: MessengerGroup;
    onback: () => void;
  }
  let { group, onback }: Props = $props();

  type Tab = 'queue' | 'log' | 'settings';
  let tab = $state<Tab>('queue');
  const tabs = $derived<Tab[]>(isManager(group) ? ['queue', 'log', 'settings'] : ['queue', 'log']);
  // A moderator who lost the role, or a manager demoted, stays on what is still theirs.
  $effect(() => { if (!tabs.includes(tab)) tab = 'queue'; });
</script>

<PanelHead title={group.queue ? `${$t('msg_mod_title')} · ${group.queue}` : $t('msg_mod_title')} {onback} />

<div class="scroll">
  <div class="seg tabs" role="tablist" aria-label={$t('msg_mod_title')}>
    {#each tabs as v (v)}
      <button class="seg-btn" class:active={tab === v} role="tab" aria-selected={tab === v} onclick={() => (tab = v)}>
        {$t(`msg_mod_tab_${v}` as 'msg_mod_tab_queue')}
      </button>
    {/each}
  </div>

  {#if tab === 'queue'}<QueueTab {group} />
  {:else if tab === 'log'}<LogTab {group} />
  {:else}<SettingsTab {group} />{/if}
</div>

<style>
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  .tabs { align-self: stretch; }
  .tabs .seg-btn { flex: 1 1 0; justify-content: center; }
</style>
