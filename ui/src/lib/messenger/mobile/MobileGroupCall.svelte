<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's page of the room of a group call. It is opened for every
  room I start or join (CallWatcher). When the room is over and that was
  shown, the page goes back to where the user was, or to the group's chat.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import { groupCallStore } from '../calls/groupCallStore.svelte';
  import GroupCallScreen from '../calls/GroupCallScreen.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, chatHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  /** The chat of the group of the room on this page, for the way back. */
  let chatId = $state<string | null>(null);
  let settled = $state(false);

  onMount(() => {
    (async () => {
      if (!messengerStore.loaded) await messengerStore.refresh().catch(() => {});
      await messengerStore.startListeners().catch(() => {});
      if (!groupCallStore.call) await groupCallStore.load().catch(() => {});
      settled = true;
    })();
  });

  $effect(() => {
    const c = groupCallStore.call ?? groupCallStore.over?.call;
    if (c) chatId = c.chat_id;
  });

  let left = false;
  function leave() {
    if (left) return;
    left = true;
    if (history.length > 1) history.back();
    else void goto(chatId ? chatHref(chatId) : BASE, { replaceState: true });
  }

  // Nothing to show any more: the end was shown, or no room was ever here.
  $effect(() => {
    if (settled && !groupCallStore.call && !groupCallStore.over) leave();
  });
</script>

{#if groupCallStore.call || groupCallStore.over}
  <MobileFrame bare scroll={false}>
    <GroupCallScreen onleave={leave} />
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_title')} onback={leave}>
    <div class="note">{$t('loading')}</div>
  </MobileFrame>
{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); }
</style>
