<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's page of a call. It is opened for every call that rings or
  is made (CallWatcher), also when the call service's Answer started the
  app: the call is already taken then, and this page shows it talking.
  When the call is over and its ending was shown, the page goes back to
  where the user was, or to the call's chat when the call opened the app.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import { callStore } from '../calls/callStore.svelte';
  import CallScreen from '../calls/CallScreen.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, chatHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  /** The chat of the call on this page, for the way back. */
  let chatId = $state<string | null>(null);
  let settled = $state(false);

  onMount(() => {
    (async () => {
      if (!messengerStore.loaded) await messengerStore.refresh().catch(() => {});
      await messengerStore.startListeners().catch(() => {});
      if (!callStore.call) await callStore.load().catch(() => {});
      settled = true;
    })();
  });

  $effect(() => {
    const c = callStore.call ?? callStore.ended?.call;
    if (c) chatId = c.chat_id;
  });

  let left = false;
  function leave() {
    if (left) return;
    left = true;
    if (history.length > 1) history.back();
    else void goto(chatId ? chatHref(chatId) : BASE, { replaceState: true });
  }

  // Nothing to show any more: the ending was shown, or no call was ever here.
  $effect(() => {
    if (settled && !callStore.call && !callStore.ended) leave();
  });
</script>

{#if callStore.call || callStore.ended}
  <MobileFrame bare scroll={false}>
    <CallScreen onleave={leave} />
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_title')} onback={leave}>
    <div class="note">{$t('loading')}</div>
  </MobileFrame>
{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); }
</style>
