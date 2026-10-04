<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import DmChat from '../dm/DmChat.svelte';
  import GroupChat from "../groups/GroupChat.svelte";
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, chatHref } from './routes';
  import { onChatOpened } from '../content/actions';
  import { pushSeen, startPushBridge } from '../push/bridge';
  import { markPhone } from '../shared/phone';

  markPhone();

  const id = $derived(page.url.searchParams.get('id') ?? '');
  let missing = $state(false);

  function back() {
    chatStore.close();
    if (history.length > 1) history.back();
    else goto(BASE, { replaceState: true });
  }

  async function load(chatId: string) {
    missing = false;
    if (!messengerStore.loaded) await messengerStore.refresh().catch(() => {});
    await messengerStore.startListeners().catch(() => {});
    if (!chatStore.chats.length) await chatStore.loadChats().catch(() => {});
    if (!chatStore.chats.some((c) => c.id === chatId)) { missing = true; return; }
    if (chatStore.activeId !== chatId) await chatStore.open(chatId);
    // What the notification was about is on the screen now. A tap may have
    // opened this page before the list of chats was ever shown.
    startPushBridge().then(() => pushSeen(chatId)).catch(() => {});
  }

  $effect(() => { if (id) load(id); });

  // A card opened another chat: a phone shows a chat on its own page.
  onMount(() => onChatOpened((chatId) => { if (chatId !== id) goto(chatHref(chatId)); }));

  onMount(() => {
    // Coming back from the background: catch up on what arrived meanwhile.
    const onVisible = () => {
      if (document.visibilityState === 'visible' && chatStore.activeId) {
        chatStore.reloadWindow().catch(() => {});
        chatStore.markRead(chatStore.activeId).catch(() => {});
      }
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  });
</script>

{#if chatStore.active && chatStore.active.id === id}
  <MobileFrame bare scroll={false}>
    {#if chatStore.active.kind === "group"}
      <GroupChat chat={chatStore.active} onback={back} />
    {:else}
      <DmChat chat={chatStore.active} onback={back} />
    {/if}
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_title')} onback={back}>
    <div class="note">{missing ? $t('msg_mobile_chat_missing') : $t('loading')}</div>
  </MobileFrame>
{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); }
</style>
