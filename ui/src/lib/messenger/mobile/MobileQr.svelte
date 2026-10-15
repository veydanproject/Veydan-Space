<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The phone's «Мой QR»: my code to show, the camera for someone else's, and the key and code by hand when the camera cannot. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import NewChatDialog from '../chats/NewChatDialog.svelte';
  import MyQr from '../invites/MyQr.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, chatHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  let manual = $state(false);

  onMount(() => {
    messengerStore.ensureLoaded().catch(() => {});
    messengerStore.startListeners().catch(() => {});
  });
</script>

<MobileFrame title={$t('msg_inv_my_qr')} onback={() => void leave(BASE)}>
  <div class="pad">
    <MyQr onchat={(id) => goto(chatHref(id))} onmanual={() => (manual = true)} />
  </div>
</MobileFrame>

<NewChatDialog bind:open={manual} onopened={() => { if (chatStore.activeId) goto(chatHref(chatStore.activeId)); }} />

<style>
  .pad { padding: var(--sp-4); }
</style>
