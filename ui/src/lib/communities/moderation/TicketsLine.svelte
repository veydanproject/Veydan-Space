<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Билеты: 12 / запросить» (communities-wire §17): how many tickets this
  moderator holds to let a newcomer read at once while the keeper is away,
  and a batch asked of the keeper. One line, nothing more.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { channelsApi, type ChannelDetail, type TicketsView } from '../api';
  import { channelStore } from '../channelStore.svelte';
  import { channelError } from '../errors';
  import { ticketsToAsk } from '../tickets';

  let { detail }: { detail: ChannelDetail } = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const cid = $derived(detail.channel.id);
  let tickets = $state<TicketsView | null>(null);
  let asked = $state(false);
  let error = $state('');

  $effect(() => {
    const id = cid;
    // The keeper issues tickets and the journal spends them: read again with their news, not at every message.
    void channelStore.keeperRev[id];
    void channelStore.journalRev[id];
    channelsApi.tickets.held(id).then((v) => { if (id === cid) tickets = v; }).catch(() => { tickets = null; });
  });

  async function ask() {
    error = '';
    try {
      tickets = await channelsApi.tickets.request(cid, ticketsToAsk(tickets));
      asked = true;
    } catch (e) {
      error = channelError(e, tr);
    }
  }
</script>

<div class="tickets" title={$t('chan_tickets_hint')}>
  <Icon name="tag" size={13} />
  <span class="count">{$t('chan_tickets', { n: String(tickets?.held ?? 0) })}</span>
  {#if ticketsToAsk(tickets) > 0}
    <span class="sep">/</span>
    <button type="button" class="link" disabled={asked} onclick={ask}>{asked ? $t('chan_tickets_asked') : $t('chan_tickets_ask')}</button>
  {/if}
  {#if error}<span class="error" role="alert">{error}</span>{/if}
</div>

<style>
  .tickets { display: flex; align-items: center; gap: 6px; padding: 6px 2px 10px; font-size: var(--fs-xs); color: var(--text-2); flex-wrap: wrap; }
  .count { font-variant-numeric: tabular-nums; }
  .sep { color: var(--text-3); }
  .link { background: none; border: 0; padding: 0; color: var(--accent); font: inherit; cursor: pointer; }
  .link:disabled { color: var(--text-3); cursor: default; }
  .error { color: var(--danger-text, var(--danger)); width: 100%; }
</style>
