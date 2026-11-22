<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The tab «Звонки» of a phone: the journal of calls (calls/CallHistory), a
  row's details in a sheet, «Новый звонок» on the round button. While the
  tab is on the screen its missed calls are no longer new.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { messengerStore } from '../store.svelte';
  import { netDot, netState as netStateOf } from '../shared/net-state';
  import { markPhone } from '../shared/phone';
  import CallHistory from '../calls/CallHistory.svelte';
  import CallDetails from '../calls/CallDetails.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import NewCallDialog from '../calls/NewCallDialog.svelte';
  import Dialer from '../calls/Dialer.svelte';
  import { callLog } from '../calls/callLog.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import MeButton from './MeButton.svelte';
  import { chatHref } from './routes';

  markPhone();

  let details = $state<string | null>(null);
  let newCall = $state(false);

  onMount(() => {
    messengerStore.refresh().catch(() => {});
    messengerStore.startListeners().catch(() => {});
    callLog.show(true).catch(() => {});
    return () => { callLog.shown = false; };
  });

  const s = $derived(messengerStore.status);
  const connected = $derived(s?.runtime?.relays_connected ?? 0);
  const total = $derived(s?.runtime?.relays_total ?? 0);
  const netState = $derived(netStateOf(s?.runtime));

  function write(chatId: string) {
    details = null;
    void goto(chatHref(chatId));
  }
</script>

{#snippet lead()}
  <span class="status-dot {netDot(netState)}" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}></span>
{/snippet}

{#snippet actions()}
  {#if s?.runtime}<MeButton />{/if}
{/snippet}

<MobileFrame hubButton title={$t('msg_nav_calls')} {lead} {actions} scroll={false}>
  <div class="calls">
    <CallHistory onselect={(r) => (details = r.head.chat_id)} onchat={write} onnew={() => (newCall = true)} />
  </div>
  <button class="m-fab" onclick={() => (newCall = true)} aria-label={$t('msg_calls_new')}><Icon name="phone-plus" size={22} /></button>
</MobileFrame>

<Dialog open={!!details} onclose={() => (details = null)}>
  {#if details}<CallDetails chatId={details} onchat={write} oncall={() => (details = null)} ongone={() => (details = null)} />{/if}
</Dialog>
<NewCallDialog bind:open={newCall} />
<Dialer />

<style>
  .calls { flex: 1; min-height: 0; display: flex; flex-direction: column; padding-top: var(--sp-2); }
  /* The list scrolls its last row out from under the «+» (.m-fab). */
  .calls :global(.list-wrap) { padding-bottom: var(--fab-room); }
</style>
