<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A conversation of two: the chat window and, when asked for, the panel
  about the person beside it (a tap on the face or the name in the header;
  the menu of the chat has no entry for it, the owner's word 2026-10-18).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import ChatWindow from './ChatWindow.svelte';
  import PeerInfo from './PeerInfo.svelte';
  import CallButton from '../calls/CallButton.svelte';
  import WithInfo from '../shared/WithInfo.svelte';
  import { onPhone } from '../shared/phone';
  import type { MessengerChat } from '../api';

  interface Props {
    chat: MessengerChat;
    onback?: () => void;
  }
  let { chat, onback }: Props = $props();

  let info = $state(false);

  // The panel belongs to the chat it was opened for.
  let last = '';
  $effect(() => {
    if (chat.id !== last) { last = chat.id; info = false; }
  });

  /** On a phone the header keeps «Call» and ⋮ alone: «Video call» and «Search in chat» are the first entries of the chat's menu (design 15). */
  const phone = onPhone();
  let callButton = $state<ReturnType<typeof CallButton>>();
  let windowRef = $state<ReturnType<typeof ChatWindow>>();

  const lead = $derived.by((): MenuEntry[] => {
    const list: MenuEntry[] = [];
    if (!phone) return list;
    if (callButton?.canCall()) {
      list.push({ label: $t('msg_call_video_start'), icon: 'video', disabled: !!callButton.callBlocked(), onselect: () => callButton?.start('video') });
    }
    list.push({ label: $t('msg_search_open'), icon: 'search', onselect: () => windowRef?.search() }, { type: 'separator' });
    return list;
  });
</script>

{#snippet actions()}
  <CallButton bind:this={callButton} {chat} videoInMenu={phone} />
{/snippet}

{#snippet window()}
  <ChatWindow bind:this={windowRef} {chat} {onback} {actions} leadEntries={lead} menuWidth={288} ontitle={() => (info = !info)} />
{/snippet}

{#snippet panel()}
  <PeerInfo {chat} onclose={() => (info = false)} />
{/snippet}

<WithInfo open={info} chat={window} info={panel} />
