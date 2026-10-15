<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One request of a stranger on the phone, by their key (`?peer=`). Accepted,
  it gives way to the chat it became; declined or blocked, back to the list.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import RequestView from '../requests/RequestView.svelte';
  import { requestStore } from '../requests/requestStore.svelte';
  import type { RequestView as Request } from '../api';
  import MobileFrame from './MobileFrame.svelte';
  import { chatHref, requestsHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  const peer = $derived(page.url.searchParams.get('peer') ?? '');
  /** Read once the store could have it: before that the page says it is loading, not that it is gone. */
  let checked = $state(false);
  /**
   * The request as last seen: an answer takes it out of the store before
   * the page moves on, and the page keeps showing it until then. Gone for
   * any other reason (it ran out, another device answered it, a reload
   * lost it), the page says so instead of offering buttons that would fail.
   */
  let shown = $state<Request | null>(null);
  /** An answer to the shown request is under way or just given here. */
  let answering = false;
  $effect(() => {
    const r = requestStore.find(peer);
    const acting = requestStore.acting === peer;
    const key = peer;
    untrack(() => {
      if (acting) answering = true;
      if (r) {
        shown = r;
        answering = false;
      } else if (shown?.peer !== key || !answering) {
        shown = null;
      }
    });
  });

  onMount(async () => {
    await messengerStore.ensureLoaded().catch(() => {});
    messengerStore.startListeners().catch(() => {});
    // The key as people pass it on is in the row of its chat.
    if (!chatStore.chats.length) await chatStore.loadChats().catch(() => {});
    await requestStore.load().catch(() => {});
    checked = true;
  });

  const back = () => void leave(requestsHref());
  const accepted = (chatId: string) => void goto(chatHref(chatId), { replaceState: true });
</script>

{#if shown}
  <MobileFrame bare scroll={false}>
    <RequestView request={shown} onback={back} onaccepted={accepted} ondone={back} />
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_req_title')} onback={back}>
    <p class="note">{checked ? $t('msg_req_gone') : $t('loading')}</p>
  </MobileFrame>
{/if}

<style>
  .note { margin: 0; padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); line-height: 1.5; }
</style>
