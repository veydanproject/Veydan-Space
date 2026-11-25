<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import { split } from '$lib/core/ui/split.svelte';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import DmChat from '../dm/DmChat.svelte';
  import GroupChat from "../groups/GroupChat.svelte";
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, openHref } from './routes';
  import { chatSections } from '../seams/sections.svelte';
  import { shouldLoad, wasRemoved } from './chatLoad';
  import { onChatOpened } from '../content/actions';
  import { pushSeen, startPushBridge } from '../push/bridge';
  import { markPhone } from '../shared/phone';

  markPhone();

  const id = $derived(page.url.searchParams.get('id') ?? '');
  /** The chat opens filtered by this search (`chatHref(id, query)`): asked once per address. */
  const query = $derived(page.url.searchParams.get('q') ?? '');
  let searchedFor = '';
  $effect(() => {
    const key = `${id}\n${query}`;
    if (!id || !query || searchedFor === key) return;
    searchedFor = key;
    chatStore.openSearch = { chatId: id, query };
  });
  /** The chat that is not on this device. */
  let missingId = $state<string | null>(null);
  let loading = $state(false);
  /** The chat this page loaded last: a chat new to the page is loaded once. */
  let loadedFor: string | null = null;
  /** The resets of the store when this page loaded its chat. */
  let loadedEpoch = 0;
  const known = $derived(chatStore.chats.some((c) => c.id === id));
  const shown = $derived(chatStore.active?.id === id);

  // The chat stays open while history moves: a back that lands on this chat
  // again, or nowhere, never shows an empty page. It is closed when the page
  // is really left (onDestroy below). Beside its list (a wide window) the
  // arrow is not shown, but a deleted chat still goes: to its list's address.
  function back() {
    void leave(split.of(page.url)?.home ?? BASE);
  }

  // A page still waiting for its chat goes back where it was opened from.
  function toList() {
    void leave(BASE);
  }

  /** How long the page waits for its chat before it says the chat is not here. */
  const STUCK_MS = 10_000;
  let stuck = $state(false);
  $effect(() => {
    void id;
    stuck = false;
    if (shown) return;
    const timer = setTimeout(() => (stuck = true), STUCK_MS);
    return () => clearTimeout(timer);
  });

  async function load(chatId: string) {
    loadedFor = chatId;
    loadedEpoch = chatStore.resets;
    loading = true;
    try {
      await open(chatId);
    } finally {
      loading = false;
    }
  }

  async function open(chatId: string) {
    if (missingId === chatId) missingId = null;
    if (!messengerStore.loaded) await messengerStore.refresh().catch(() => {});
    await messengerStore.startListeners().catch(() => {});
    if (!chatStore.chats.length) await chatStore.loadChats().catch(() => {});
    if (!chatStore.chats.some((c) => c.id === chatId)) { missingId = chatId; return; }
    if (chatStore.activeId !== chatId) await chatStore.open(chatId);
    // What the notification was about is on the screen now. A tap may have
    // opened this page before the list of chats was ever shown.
    startPushBridge().then(() => pushSeen(chatId)).catch(() => {});
  }

  // A chat new to the page is loaded. So is this one again when the store
  // was emptied under the page (the app came back locked) or the chat turned
  // up after it was missing; a chat closed on purpose (deleted, forgotten,
  // archived) is not reopened. What load reads stays out of this effect: it
  // would run again at every change of the stores.
  $effect(() => {
    const go = shouldLoad({
      id, loading, loadedFor, loadedEpoch, resets: chatStore.resets,
      active: !!chatStore.active, missingId, known,
    });
    if (go) untrack(() => load(id));
  });

  // Beside its list (a wide window) the chat shown may be deleted from the
  // list's menu, the last one too: the right part empties, as after a delete
  // from the chat's own menu (back; `leave` goes once when both ask). A store
  // emptied under the page (`resets`) is not a delete: the chat comes back.
  let shownFor = '';
  let shownEpoch = 0;
  $effect(() => {
    if (shown) {
      shownFor = id;
      shownEpoch = chatStore.resets;
    }
    if (shownFor !== id || known) return;
    shownFor = '';
    if (split.on && wasRemoved({ known, shownEpoch, resets: chatStore.resets })) untrack(back);
  });

  // Whatever the page is left for (the list, contacts, Notes, the system
  // back), its chat is closed: an open chat marks what arrives as read. The
  // chat the page loaded, not the address, which may already be the next page.
  // A move to another chat keeps this page and does not come here.
  onDestroy(() => {
    if (loadedFor && chatStore.activeId === loadedFor) chatStore.close();
  });

  // A card opened another chat: a phone shows a chat on its own page. It
  // takes this chat's place, so chats opened one from another never pile up
  // backs: a back leads to the list. Opened from a group's members (`stack`),
  // the group stays behind it and a back returns there.
  onMount(() => onChatOpened((chatId, { stack }) => {
    if (chatId !== id) goto(openHref(chatId, !!chatSections.ofChat(chatId)), { replaceState: !stack });
  }));

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

{#if chatStore.active && shown}
  <MobileFrame bare scroll={false}>
    {#if chatStore.active.kind === "group"}
      <GroupChat chat={chatStore.active} onback={back} />
    {:else}
      <DmChat chat={chatStore.active} onback={back} />
    {/if}
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_title')} onback={split.on ? undefined : toList}>
    {#if missingId === id || stuck}
      <div class="note">{$t('msg_mobile_chat_missing')}</div>
    {:else}
      <div class="note">{$t('loading')}</div>
    {/if}
  </MobileFrame>
{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); }
</style>
