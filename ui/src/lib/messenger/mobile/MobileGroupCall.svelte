<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The phone's page of the room of a group call. It is opened for every
  room I start or join (CallWatcher). When the room is over and that was
  shown, the page goes back to where the user was, or to the group's chat.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { afterNavigate, goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { leaveOnce } from '$lib/core/ui/back';
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

  // Once: the arrow and the end of the call both ask. A back that went
  // nowhere goes to the chat instead, and the arrow stays alive.
  const leave = leaveOnce(() => (chatId ? chatHref(chatId) : BASE));

  /** The page this one was come to from: the room's own chat is gone back to, not opened twice. */
  let from = $state<string | null>(null);
  afterNavigate((nav) => { from = nav.from ? nav.from.url.pathname + nav.from.url.search : null; });

  /** «Открыть чат»: the chat takes this page's place in the history (the room goes on, «Вернуться к звонку» comes back
      here); a back from the chat goes where the user was before the room, not to its page again. */
  function openChat() {
    if (!chatId) return;
    if (from === chatHref(chatId)) void leave();
    else void goto(chatHref(chatId), { replaceState: true });
  }

  // Nothing to show any more: the end was shown, or no room was ever here.
  $effect(() => {
    if (settled && !groupCallStore.call && !groupCallStore.over) leave();
  });
</script>

{#if groupCallStore.call || groupCallStore.over}
  <MobileFrame bare scroll={false}>
    <GroupCallScreen onleave={leave} onchat={chatId ? openChat : undefined} />
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_title')} onback={leave}>
    <div class="note">{$t('loading')}</div>
  </MobileFrame>
{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); }
</style>
