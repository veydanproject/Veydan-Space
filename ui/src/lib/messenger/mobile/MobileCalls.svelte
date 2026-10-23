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
  import { netState as netStateOf } from '../shared/net-state';
  import { markPhone } from '../shared/phone';
  import CallHistory from '../calls/CallHistory.svelte';
  import CallDetails from '../calls/CallDetails.svelte';
  import CallIcon from '../calls/CallIcon.svelte';
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
  <span class="net {netState}" title={$t(`msg_net_${netState}` as 'msg_net_on', { connected: String(connected), total: String(total) })}></span>
{/snippet}

{#snippet actions()}
  {#if s?.runtime}<MeButton />{/if}
{/snippet}

<MobileFrame hubButton title={$t('msg_nav_calls')} {lead} {actions} scroll={false}>
  <div class="calls">
    <CallHistory onselect={(r) => (details = r.head.chat_id)} onchat={write} onnew={() => (newCall = true)} />
  </div>
  <button class="fab" onclick={() => (newCall = true)} aria-label={$t('msg_calls_new')}><CallIcon name="phone" size={22} /></button>
</MobileFrame>

<Dialog open={!!details} onclose={() => (details = null)}>
  {#if details}<CallDetails chatId={details} onchat={write} oncall={() => (details = null)} ongone={() => (details = null)} />{/if}
</Dialog>
<NewCallDialog bind:open={newCall} />
<Dialer />

<style>
  .calls { flex: 1; min-height: 0; display: flex; flex-direction: column; padding-top: var(--sp-2); }
  .net { width: 9px; height: 9px; border-radius: 50%; margin-left: 8px; background: var(--text-3); flex-shrink: 0; }
  .net.on { background: var(--success); box-shadow: 0 0 0 3px var(--success-bg); }
  .net.connecting { background: var(--warn-text); animation: pulse 1.4s ease-in-out infinite; }
  .net.lost { background: var(--danger); }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .fab {
    position: absolute; right: var(--sp-4); bottom: var(--sp-4); width: 56px; height: 56px; border: none; border-radius: 50%;
    background: var(--accent-grad); color: #fff; display: inline-flex; align-items: center; justify-content: center;
    box-shadow: var(--shadow-accent);
  }
  .fab:active { filter: brightness(0.92); }
</style>
