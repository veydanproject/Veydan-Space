<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A conversation of two: the chat window and, when asked for, the panel
  about the person beside it (a tap on the face or the name in the header,
  or its entry in the menu of the chat).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import type { MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import ChatWindow from './ChatWindow.svelte';
  import PeerInfo from './PeerInfo.svelte';
  import CallButton from '../calls/CallButton.svelte';
  import WithInfo from '../shared/WithInfo.svelte';
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

  const lead = $derived<MenuEntry[]>([{ label: $t('msg_peer_info'), icon: 'user', onselect: () => (info = true) }]);
</script>

{#snippet actions()}
  <CallButton {chat} />
{/snippet}

{#snippet window()}
  <ChatWindow {chat} {onback} {actions} leadEntries={lead} ontitle={() => (info = !info)} />
{/snippet}

{#snippet panel()}
  <PeerInfo {chat} onclose={() => (info = false)} />
{/snippet}

<WithInfo open={info} chat={window} info={panel} />
